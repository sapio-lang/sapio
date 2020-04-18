from collections import defaultdict
from typing import List, Tuple, Union, Type, DefaultDict, cast

from sapio.bitcoinlib.script import CScript
from sapio.script.clause import Clause, AfterClause, RelativeTimeSpec, AbsoluteTimeSpec, Variable, \
    UnsatisfiableClause, SatisfiedClause, CheckTemplateVerifyClause, DNFClause, TimeSpec
from sapio.script.clause_to_fragment import FragmentCompiler
from sapio.script.flatten_and import FlattenPass
from sapio.script.normalize_or import NormalizationPass
from sapio.script.opcodes import AllowedOp
from sapio.script.witnessmanager import WitnessTemplate, WitnessManager

DNF = List[List[DNFClause]]


class ClauseToDNF:
    def compile_cnf(self, clause: Clause) -> DNF:
        while True:
            normalizer = NormalizationPass()
            clause = normalizer.normalize(clause)
            if not normalizer.took_action:
                break
        return FlattenPass().flatten(clause)


import logging


class AfterClauseSimplification:
    ReturnType = Union[UnsatisfiableClause, Tuple[
        Union[SatisfiedClause, AfterClause],
        Union[SatisfiedClause, AfterClause]]]
    PRUNE_MODE: bool = True

    def simplify(self, clauses: List[AfterClause]) -> ReturnType:

        log = logging.getLogger("compiler").getChild(self.__class__.__name__)
        relative_or_absolute : DefaultDict[Type[TimeSpec], List[TimeSpec]] = defaultdict(list)
        ret : List[Union[SatisfiedClause, AfterClause]]= [SatisfiedClause(), SatisfiedClause()]
        for cl in clauses:
            assert cl.a.assigned_value is not None
            relative_or_absolute[type(cl.a.assigned_value)].append(cl.a.assigned_value)
        relative = relative_or_absolute[RelativeTimeSpec]
        absolute = relative_or_absolute[AbsoluteTimeSpec]
        relative_blocks_or_time = defaultdict(list)
        for cl2 in relative:
            relative_blocks_or_time[cl2.get_type()].append(cl2)
        relative_blocks = relative_blocks_or_time[RelativeTimeSpec.Blocks]
        relative_time = relative_blocks_or_time[RelativeTimeSpec.Time]
        if not ((len(relative_time) > 0) ^ (len(relative_blocks) > 0) or not (relative_blocks or relative_time)):
            # Todo: Is this a true error? Or can we simply safely prune this branch...
            if self.PRUNE_MODE:
                log.warning("Incompatible Relative Time Locks! Pruning Branch")
                return UnsatisfiableClause()
            else:
                raise AssertionError("Incompatible Relative Time Locks in Branch")
        elif relative_blocks or relative_time:
            (_, tl) = max((tl.time, tl) for tl in relative_blocks + relative_time)
            ret[0] = AfterClause(Variable("relative_time_lock", tl))

        absolute_blocks_or_time = defaultdict(list)
        for cl3 in absolute:
            absolute_blocks_or_time[cl3.get_type()].append(cl3)
        absolute_blocks = absolute_blocks_or_time[AbsoluteTimeSpec.Blocks]
        absolute_time = absolute_blocks_or_time[AbsoluteTimeSpec.Time]
        if not ((len(absolute_time) > 0) ^ (len(absolute_blocks) > 0) or not (absolute_time or absolute_blocks)):
            # Todo: Is this a true error? Or can we simply safely prune this branch...
            if self.PRUNE_MODE:
                log.warning("Incompatible Absolute Time Locks! Pruning Branch")
                return UnsatisfiableClause()
            else:
                raise AssertionError("Incompatible Absolute Time Locks in Branch")
        elif absolute_time or absolute_blocks:
            (_, tl) = max((tl.time, tl) for tl in absolute_blocks + absolute_time)
            ret[1] = AfterClause(Variable("absolute_time_lock", tl))
        return (ret[0], ret[1])


class DNFSimplification:
    PRUNE_MODE: bool = True

    def simplify(self, all_clauses: List[DNFClause]) -> List[DNFClause]:
        clauses_to_return : List[DNFClause] = []
        log = logging.getLogger("compiler").getChild(self.__class__.__name__)
        clause_by_type = defaultdict(list)
        for cl in all_clauses:
            clause_by_type[type(cl)].append(cl)

        if AfterClause in clause_by_type:
            after_clauses = cast(List[AfterClause], clause_by_type.pop(AfterClause))
            val = AfterClauseSimplification().simplify(after_clauses)
            if isinstance(val, tuple):
                (a, b) = val
                if not isinstance(a, SatisfiedClause):
                    clauses_to_return.append(a)
                if not isinstance(b, SatisfiedClause):
                    clauses_to_return.append(b)
            else:
                return [UnsatisfiableClause()]
        if CheckTemplateVerifyClause in clause_by_type:
            ctv_clauses = cast(List[CheckTemplateVerifyClause], clause_by_type.pop(CheckTemplateVerifyClause))
            if len(ctv_clauses) <= 1:
                clauses_to_return.extend(list(ctv_clauses))
            else:
                first = ctv_clauses[0].a.assigned_value
                if not all(clause.a.assigned_value == first for clause in ctv_clauses):
                    if self.PRUNE_MODE:
                        log.warning("Pruning Incompatible CheckTemplateVerify")
                        return [UnsatisfiableClause()]
                    else:
                        raise AssertionError("Incompatible CheckTemplateVerify Clause")
                else:
                    clauses_to_return.append(ctv_clauses[0])

        for (type_, clauses) in clause_by_type.items():
            clauses_to_return += clauses

        return clauses_to_return


class CNFClauseCompiler:
    def compile(self, cl: List[DNFClause], w: WitnessTemplate) -> CScript:
        return CScript([FragmentCompiler()._compile(frag, w) for frag in cl])


class ProgramBuilder:

    def compile(self, clause: Clause) -> WitnessManager:
        dnf: DNF = ClauseToDNF().compile_cnf(clause)
        n_cases = len(dnf)
        witness_manager: WitnessManager = WitnessManager()
        dnf = list(filter(lambda x: not any(isinstance(y, UnsatisfiableClause) for y in x),
                          (DNFSimplification().simplify(x) for x in dnf)))
        # If we have one or two cases, special case the emitted scripts
        # 3 or more, use a generic wrapper
        if n_cases == 1:
            witness = witness_manager.make_witness(0)
            witness_manager.program += CNFClauseCompiler().compile(dnf[0], witness)
            # Hack because the fragment compiler leaves stack empty
            witness_manager.program += CScript([AllowedOp.OP_1])
        elif n_cases == 2:
            wit_0 = witness_manager.make_witness(0)
            wit_1 = witness_manager.make_witness(1)
            wit_0.add(1)
            wit_1.add(0)
            # note order of side effects!
            branch_a = CNFClauseCompiler().compile(dnf[0], wit_0)
            branch_b = CNFClauseCompiler().compile(dnf[1], wit_1)
            witness_manager.program = CScript([AllowedOp.OP_IF,
                                               branch_a,
                                               AllowedOp.OP_ELSE,
                                               branch_b,
                                               AllowedOp.OP_ENDIF,
                                               AllowedOp.OP_1])
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
                sub_script = CNFClauseCompiler().compile(cl, wit)
                if idx == 0:
                    witness_manager.program = \
                        CScript([
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
                            AllowedOp.OP_1SUB])
                elif idx + 1 < len(dnf):
                    witness_manager.program += \
                        CScript([AllowedOp.OP_IFDUP,
                                 AllowedOp.OP_NOTIF,
                                 sub_script,
                                 AllowedOp.OP_0,
                                 AllowedOp.OP_ENDIF,
                                 AllowedOp.OP_1SUB])
                # Last clause!
                else:
                    # No ifdup required since we are last, no need for data on
                    # stack
                    # End with an OP_1 so that we succeed after all cases
                    witness_manager.program += \
                        CScript([AllowedOp.OP_NOTIF,
                                 sub_script,
                                 AllowedOp.OP_ENDIF,
                                 AllowedOp.OP_1])

        return witness_manager
