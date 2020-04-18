from sapio.bitcoinlib.hash_functions import sha256
from sapio.bitcoinlib.script import CScript
from sapio.spending_conditions.opcodes import AllowedOp
from sapio.spending_conditions.witnessmanager import CTVHash, WitnessTemplate
from sapio.spending_conditions.script_lang import Clause, SignatureCheckClause, PreImageCheckClause, \
    CheckTemplateVerifyClause, AfterClause, AbsoluteTimeSpec, RelativeTimeSpec, Variable
from sapio.util import methdispatch


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