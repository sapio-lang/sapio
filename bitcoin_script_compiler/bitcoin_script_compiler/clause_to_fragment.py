from functools import singledispatchmethod
from typing import TYPE_CHECKING, Any, ClassVar, Union

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
from .unassigned import PreImageVar, SignatureVar
from .variable import AssignedVariable
from .witnessmanager import CTVHash, WitnessTemplate


class FragmentCompiler:
    """FragmentCompiler emits CScript fragments for DNFClauses. Fragments always
    leave the stack clean.  FragmentCompiler also populates data into the
r   WitnessTemplate."""
    def __call__(self, arg: Clause, witness: WitnessTemplate) -> CScript:
        """Convert a clause to CScript Fragment"""
        if TYPE_CHECKING:
            assert callable(self._compile)
        r: CScript = self._compile(arg, witness)
        return r

    @singledispatchmethod
    def _compile(self, arg: Clause, witness: WitnessTemplate) -> CScript:
        raise NotImplementedError("Cannot Compile Arg", arg)

    @_compile.register
    def _compile_signature(
        self, arg: SignatureCheckClause, witness: WitnessTemplate
    ) -> CScript:
        if TYPE_CHECKING:
            assert callable(self._compile)
        witness.add(SignatureVar(arg))
        script: CScript = self._compile(arg.a, witness) + CScript(
            [AllowedOp.OP_CHECKSIGVERIFY]
        )
        return script

    @_compile.register
    def _compile_preimage(
        self, arg: PreImageCheckClause, witness: WitnessTemplate
    ) -> CScript:
        if TYPE_CHECKING:
            assert callable(self._compile)
        witness.add(PreImageVar(arg))
        script: CScript = CScript([AllowedOp.OP_SHA256]) + self._compile(
            arg.a, witness
        ) + CScript([AllowedOp.OP_EQUALVERIFY])
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

    @_compile.register
    def _compile_assigned_var(
        self, arg: AssignedVariable, witness: WitnessTemplate
    ) -> CScript:
        return CScript([arg.assigned_value])
