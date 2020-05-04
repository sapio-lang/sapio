from functools import singledispatchmethod
from typing import TYPE_CHECKING, Any, ClassVar

from bitcoinlib.hash_functions import sha256
from bitcoinlib.script import CScript

from .clause import (
    AbsoluteTimeSpec,
    AfterClause,
    CheckTemplateVerifyClause,
    Clause,
    PreImageCheckClause,
    RelativeTimeSpec,
    SignatureCheckClause,
)
from .opcodes import AllowedOp
from .variable import AssignedVariable, UnassignedVariable
from .witnessmanager import CTVHash, WitnessTemplate


class FragmentCompiler:
    def __call__(self, arg: Clause, witness: WitnessTemplate) -> CScript:
        if TYPE_CHECKING:
            assert callable(self._compile)
        return self._compile(arg, witness)

    @singledispatchmethod
    def _compile(self, arg: Clause, witness: WitnessTemplate) -> CScript:
        raise NotImplementedError("Cannot Compile Arg", arg)

    @_compile.register
    def _compile_and(
        self, arg: SignatureCheckClause, witness: WitnessTemplate
    ) -> CScript:
        if TYPE_CHECKING:
            assert callable(self._compile)
        variable: CScript = self._compile(
            UnassignedVariable(b"_signature_by_" + arg.a.name), witness
        )
        script: CScript = self._compile(arg.a, witness) + CScript(
            [AllowedOp.OP_CHECKSIGVERIFY]
        )
        assert len(variable) == 0
        return script

    @_compile.register
    def _compile_preimage(
        self, arg: PreImageCheckClause, witness: WitnessTemplate
    ) -> CScript:
        if TYPE_CHECKING:
            assert callable(self._compile)
        variable: CScript = self._compile(
            UnassignedVariable(b"_preimage_of_" + arg.a.name), witness
        )
        script: CScript = CScript([AllowedOp.OP_SHA256]) + self._compile(
            arg.a, witness
        ) + CScript([AllowedOp.OP_EQUALVERIFY])
        assert len(variable) == 0
        return script

    @_compile.register
    def _compile_ctv(
        self, arg: CheckTemplateVerifyClause, witness: WitnessTemplate
    ) -> CScript:
        # While valid to make this a witness variable, this is likely an error
        assert arg.a.assigned_value is not None
        assert isinstance(arg.a.assigned_value, bytes)
        witness.will_execute_ctv(CTVHash(arg.a.assigned_value))
        s = CScript(
            [arg.a.assigned_value, AllowedOp.OP_CHECKTEMPLATEVERIFY, AllowedOp.OP_DROP]
        )
        return s

    @_compile.register
    def _compile_after(self, arg: AfterClause, witness: WitnessTemplate) -> CScript:
        # While valid to make this a witness variable, this is likely an error
        assert arg.a.assigned_value is not None
        if isinstance(arg.a.assigned_value, AbsoluteTimeSpec):
            return CScript(
                [
                    arg.a.assigned_value.time,
                    AllowedOp.OP_CHECKLOCKTIMEVERIFY,
                    AllowedOp.OP_DROP,
                ]
            )
        if isinstance(arg.a.assigned_value, RelativeTimeSpec):
            return CScript(
                [
                    arg.a.assigned_value.time,
                    AllowedOp.OP_CHECKSEQUENCEVERIFY,
                    AllowedOp.OP_DROP,
                ]
            )
        raise ValueError

    PREFIX: ClassVar[bytes] = sha256(bytes(1000))

    @_compile.register
    def _compile_assigned_var(
        self, arg: AssignedVariable, witness: WitnessTemplate
    ) -> CScript:
        return CScript([arg.assigned_value])

    @_compile.register(UnassignedVariable)
    def _compile_unassigned_var(
        self, arg: UnassignedVariable, witness: WitnessTemplate
    ) -> CScript:
        witness.add(self.PREFIX + arg.name)
        return CScript()
