from typing import TYPE_CHECKING, List

from sapio_bitcoinlib.script import CScript

from .clause import DNF, Clause, DNFClause, Unsatisfiable
from .clause_to_fragment import FragmentCompiler
from .flatten_and import FlattenPass
from .normalize_or import NormalizationPass
from .opcodes import AllowedOp
from .simplify import DNFSimplification
from .witnessmanager import WitnessManager, WitnessTemplate


class ClauseToDNF:
    def compile_cnf(self, clause: Clause) -> DNF:
        """Turns a Clause into a DNF (needs to be renamed).

        Parameters
        ----------
        clause: Clause

        Returns
        -------
        DNF
            A DNF with logical equivalence to Clause"""
        while True:
            normalizer = NormalizationPass()
            clause = normalizer(clause)
            if not normalizer.took_action:
                break
        return FlattenPass()(clause)


class DNFClauseCompiler:
    def compile(self, cl: List[DNFClause], w: WitnessTemplate) -> CScript:
        """
        Turns a set of conditions into a CScript function.

        Parameters
        ----------
        cl: List[DNFClause]
            A list of base DNFClause which are to be treated as anded together
        w: WitnessTemplate
            The WitnessTemplate which will contain the ABI desciption for this function

        Returns
        -------
        CScript
            The computed function

        """
        return CScript([FragmentCompiler()(frag, w) for frag in cl])


class ProgramBuilder:
    """
    This class wraps the compile function to house future options/compilation
    parameters that may be passed in.

    After the first release, this class should honor a version flag so that deterministic
    compilation can work retroactively for older versions.
    """

    def compile(self, clause: Clause) -> WitnessManager:
        """
        compile turns a higher-order logical clause into a WitnessManager object
        which can return an address and perform transaction finalization.

        Parameters
        ----------
        clause: Clause
            The logical clause to compile to Bitcoin script.

        Returns
        -------
        WitnessManager
            An object which can be used to get addresses and get spend paths

        """
        dnf: DNF = ClauseToDNF().compile_cnf(clause)
        n_cases = len(dnf)
        witness_manager: WitnessManager = WitnessManager()
        dnf = list(
            filter(
                lambda x: not any(isinstance(y, Unsatisfiable) for y in x),
                (DNFSimplification().simplify(x) for x in dnf),
            )
        )
        # If we have one or two cases, special case the emitted scripts
        # 3 or more, use a generic wrapper
        if n_cases == 1:
            witness = witness_manager.make_witness(0)
            witness_manager.program += DNFClauseCompiler().compile(dnf[0], witness)
            # Hack because the fragment compiler leaves stack empty
            witness_manager.program += CScript([AllowedOp.OP_1])
        elif n_cases == 2:
            wit_0 = witness_manager.make_witness(0)
            wit_1 = witness_manager.make_witness(1)
            wit_0.add(1)
            wit_1.add(0)
            # note order of side effects!
            branch_a = DNFClauseCompiler().compile(dnf[0], wit_0)
            branch_b = DNFClauseCompiler().compile(dnf[1], wit_1)
            witness_manager.program = CScript(
                [
                    AllowedOp.OP_IF,
                    branch_a,
                    AllowedOp.OP_ELSE,
                    branch_b,
                    AllowedOp.OP_ENDIF,
                    AllowedOp.OP_1,
                ]
            )
        else:
            # If we have more than 3 cases, we can use a nice gadget
            # to emulate a select/jump table in Bitcoin Script.
            # It has an overhead of 5 bytes per branch.
            # Future work can optimize this by inspecting the sub-branches
            # and sharing code...

            # Check that the first argument passed is an in range execution path
            # Note the first branch does not subtract one, so we have arg in [0, N)
            for (idx, cl) in enumerate(dnf):
                wit = witness_manager.make_witness(idx)
                wit.add(idx)
                sub_script = DNFClauseCompiler().compile(cl, wit)
                if idx == 0:
                    witness_manager.program = CScript(
                        [
                            # Verify the top stack item (branch select)
                            # is in range. This is required or else a witness
                            # of e.g. n+1 could steal funds
                            AllowedOp.OP_DUP,
                            AllowedOp.OP_0,
                            n_cases,
                            AllowedOp.OP_WITHIN,
                            AllowedOp.OP_VERIFY,
                            # Successfully range-checked!
                            # If it is 0, do not duplicate as we will take the branch
                            AllowedOp.OP_IFDUP,
                            AllowedOp.OP_NOTIF,
                            sub_script,
                            # We push an OP_0 onto the stack as it will cause
                            # all following branches to not execute,
                            # unless we are the last branch
                            AllowedOp.OP_0,
                            AllowedOp.OP_ENDIF,
                            # set up for testing the next clause...
                            AllowedOp.OP_1SUB,
                        ]
                    )
                elif idx + 1 < len(dnf):
                    witness_manager.program += CScript(
                        [
                            AllowedOp.OP_IFDUP,
                            AllowedOp.OP_NOTIF,
                            sub_script,
                            AllowedOp.OP_0,
                            AllowedOp.OP_ENDIF,
                            AllowedOp.OP_1SUB,
                        ]
                    )
                # Last clause!
                else:
                    # No ifdup required since we are last, no need for data on
                    # stack
                    # End with an OP_1 so that we succeed after all cases
                    witness_manager.program += CScript(
                        [
                            AllowedOp.OP_NOTIF,
                            sub_script,
                            AllowedOp.OP_ENDIF,
                            AllowedOp.OP_1,
                        ]
                    )

        return witness_manager
