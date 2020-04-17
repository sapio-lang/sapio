from typing import TypeVar, List, Any, Dict, NewType

from sapio.bitcoinlib import segwit_addr
from sapio.bitcoinlib.address import script_to_p2wsh
from sapio.bitcoinlib.hash_functions import sha256
from sapio.bitcoinlib.script import CScript
from sapio.spending_conditions.opcodes import AllowedOp
from sapio.spending_conditions.script_lang import Variable, Clause, AndClause, AndClauseArgument, OrClause, \
    SignatureCheckClause, \
    PreImageCheckClause, CheckTemplateVerifyClause, AfterClause, AbsoluteTimeSpec, RelativeTimeSpec, SatisfiedClause
from sapio.util import methdispatch

T = TypeVar('T')


CTVHash = NewType("CTVHash", bytes)
class WitnessTemplate:
    def __init__(self):
        self.witness = []
        self.ctv_hash : Optional[CTVHash] = None
    @methdispatch
    def add(self, it : CScript):
        self.witness.insert(0, it)
    @add.register
    def _(self, it: int):
        self.add(CScript([it]))
    def will_execute_ctv(self, ctv:CTVHash):
        if self.ctv_hash is not None and ctv != self.ctv_hash:
            raise AssertionError("Two CTV Hashes cannot be in the same witness")
        self.ctv_hash = ctv



class WitnessManager:
    def __init__(self):
        self.override_program: str = None
        self.program: CScript = CScript()
        self.witnesses : Dict[Any, WitnessTemplate] = {}
        self.is_final = False
    def finalize(self):
        self.is_final = True
    def get_witness(self, key) -> List[Any]:
        assert self.is_final
        item = self.witnesses[key].witness.copy()
        item.append(self.program)
        return item
    def make_witness(self, key) -> WitnessTemplate:
        assert not self.is_final
        assert key not in self.witnesses
        self.witnesses[key] = WitnessTemplate()
        return self.witnesses[key]
    def get_p2wsh_script(self, main=False) -> CScript:
        if self.override_program is not None:
            script =  segwit_addr.decode("bc" if main else "bcrt", self.override_program)
            return CScript([script[0], bytes(script[1])])
        return CScript([AllowedOp.OP_0, sha256(self.program)])
    def get_p2wsh_address(self) -> str:
        if self.override_program is not None:
            return self.override_program
        return script_to_p2wsh(self.program)


class NormalizationPass:
    def __init__(self):
        self.took_action: bool = False
    # Normalize Bubbles up all the OR clauses into a CNF
    @methdispatch
    def normalize(self, arg: Clause) -> Clause:
        raise NotImplementedError("Cannot Compile Arg", arg)

    @normalize.register
    def normalize_and(self, arg: AndClause) -> Clause:
        a: AndClauseArgument = arg.a
        b: AndClauseArgument = arg.b
        ret = arg
        if isinstance(a, OrClause) and isinstance(b, OrClause):
            self.took_action = True
            a0: AndClauseArgument = self.normalize(a.a)
            a1: AndClauseArgument = self.normalize(a.b)
            b0: AndClauseArgument = self.normalize(b.a)
            b1: AndClauseArgument = self.normalize(b.b)
            ret = a0 * b0 + a0 * b1 + a1 * b0 + a1 * b1
        elif isinstance(b, AndClause) and isinstance(a, OrClause):
            self.took_action = True
            _or, _and = self.normalize(a), self.normalize(b)
            ret = _and * _or.a + _and * _or.b
        elif isinstance(a, AndClause) and isinstance(b, OrClause):
            self.took_action = True
            _or, _and = self.normalize(b), self.normalize(a)
            ret =_and * _or.a + _and * _or.b
        # Other Clause can be ignored...
        elif isinstance(a, AndClause):
            ret = self.normalize(a)*b
        elif isinstance(a, OrClause):
            self.took_action = True
            a0, a1 = self.normalize(a.a), self.normalize(a.b)
            ret = a0 * b + a1 * b
        elif isinstance(b, AndClause):
            ret = self.normalize(b)*a
        elif isinstance(b, OrClause):
            self.took_action = True
            b0, b1 = self.normalize(b.a), self.normalize(b.b)
            ret = b0 * a + b1 * a
        return ret

    @normalize.register
    def normalize_or(self, arg: OrClause) -> Clause:
        return self.normalize(arg.a) + self.normalize(arg.b)

    # TODO: Unionize!

    @normalize.register
    def normalize_signaturecheck(self, arg: SignatureCheckClause) -> Clause:
        return arg

    @normalize.register
    def normalize_preimagecheck(self, arg: PreImageCheckClause) -> Clause:
        return arg

    @normalize.register
    def normalize_ctv(self, arg: CheckTemplateVerifyClause) -> Clause:
        return arg

    @normalize.register
    def normalize_after(self, arg: AfterClause) -> Clause:
        return arg

    @normalize.register
    def normalize_var(self, arg: Variable) -> Clause:
        return arg

# Assumes that there is no OR which comes after an AND
class FlattenPass:
    @methdispatch
    def flatten(self, arg: Clause, or_allowed: bool=True) -> List[List[Clause]]:
        raise NotImplementedError("Cannot Compile Arg")


    @flatten.register
    def flatten_and(self, arg: AndClause, or_allowed=False) -> List[List[Clause]]:
        l = self.flatten(arg.a, or_allowed)
        l2 = self.flatten(arg.b, or_allowed)
        assert len(l) == 1
        assert len(l2) == 1
        l[0].extend(l2[0])
        return l


    @flatten.register
    def flatten_or(self, arg: OrClause, or_allowed=True) -> List[List[Clause]]:
        assert or_allowed
        return self.flatten(arg.a, or_allowed) + self.flatten(arg.b, or_allowed)


    @flatten.register
    def flatten_sigcheck(self, arg: SignatureCheckClause, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]


    @flatten.register
    def flatten_preimage(self, arg: PreImageCheckClause, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]


    @flatten.register
    def flatten_ctv(self, arg: CheckTemplateVerifyClause, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]


    @flatten.register
    def flatten_after(self, arg: AfterClause, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]


    @flatten.register
    def flatten_var(self, arg: Variable, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]

try:
    FlattenPass().flatten(AndClause(OrClause(SatisfiedClause(),SatisfiedClause()), OrClause(SatisfiedClause(), SatisfiedClause())))
    raise AssertionError("this sanity check should fail")
except AssertionError:
   pass

CNF = List[List[Clause]]
class ClauseToCNF:
    def compile_cnf(self, clause: Clause) -> CNF:
        while True:
            normalizer = NormalizationPass()
            clause = normalizer.normalize(clause)
            if not normalizer.took_action:
                break
        return FlattenPass().flatten(clause)


class FragmentCompiler:

    @methdispatch
    def _compile(self, arg: Clause, witness: WitnessTemplate) -> CScript:
        raise NotImplementedError("Cannot Compile Arg", arg)

    @_compile.register
    def _compile_and(self, arg: SignatureCheckClause, witness) -> CScript:
        return self._compile(arg.b, witness) + self._compile(arg.a, witness) + CScript([AllowedOp.OP_CHECKSIGVERIFY])

    @_compile.register
    def _compile_preimage(self, arg: PreImageCheckClause, witness) -> CScript:
        return self._compile(arg.b, witness) + \
               CScript([AllowedOp.OP_SHA256]) + self._compile(arg.a, witness) + CScript([AllowedOp.OP_EQUAL])

    @_compile.register
    def _compile_ctv(self, arg: CheckTemplateVerifyClause, witness: WitnessTemplate) -> CScript:
        # While valid to make this a witness variable, this is likely an error
        assert arg.a.assigned_value is not None
        assert isinstance(arg.a.assigned_value, bytes)
        witness.will_execute_ctv(CTVHash(arg.a.assigned_value))
        s = CScript([arg.a.assigned_value, AllowedOp.OP_CHECKTEMPLATEVERIFY, AllowedOp.OP_DROP])
        return s

    @_compile.register
    def _compile_after(self, arg: AfterClause, witness) -> CScript:
        # While valid to make this a witness variable, this is likely an error
        assert arg.a.assigned_value is not None
        if isinstance(arg.a.assigned_value, AbsoluteTimeSpec):
            return CScript([arg.a.assigned_value.time, AllowedOp.OP_CHECKLOCKTIMEVERIFY, AllowedOp.OP_DROP])
        if isinstance(arg.a.assigned_value, RelativeTimeSpec):
            return CScript([arg.a.assigned_value.time, AllowedOp.OP_CHECKSEQUENCEVERIFY, AllowedOp.OP_DROP])
        raise ValueError
    PREFIX = sha256(bytes(1000))
    @_compile.register
    def _compile_var(self, arg: Variable, witness: WitnessTemplate) -> CScript:
        PREFIX = bytes(20)
        if arg.assigned_value is None:
            witness.add(self.PREFIX+ arg.name)
            return CScript()
        else:
            return CScript([arg.assigned_value])

class CNFClauseCompiler:
    def compile(self, cl: Clause, w: WitnessTemplate) -> CScript:
        return CScript([FragmentCompiler()._compile(frag, w) for frag in cl])


class ProgramBuilder:

    def compile(self, clause: Clause) -> WitnessManager:
        cnf: CNF = ClauseToCNF().compile_cnf(clause)
        n_cases = len(cnf)
        witness_manager: WitnessManager = WitnessManager()

        # If we have one or two cases, special case the emitted scripts
        # 3 or more, use a generic wrapper
        if n_cases == 1:
            witness = witness_manager.make_witness(0)
            witness_manager.program += CNFClauseCompiler().compile(cnf[0], witness)
            # Hack because the fragment compiler leaves stack empty
            witness_manager.program += CScript([AllowedOp.OP_1])
        elif n_cases == 2:
            wit_0 = witness_manager.make_witness(0)
            wit_1 = witness_manager.make_witness(1)
            wit_0.add(1)
            wit_1.add(0)
            # note order of side effects!
            branch_a = CNFClauseCompiler().compile(cnf[0], wit_0)
            branch_b = CNFClauseCompiler().compile(cnf[1], wit_1)
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
            for (idx, cl) in enumerate(cnf):
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
                elif idx+1 < len(cnf):
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
