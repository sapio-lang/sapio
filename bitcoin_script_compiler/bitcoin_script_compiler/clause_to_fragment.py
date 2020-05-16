from functools import singledispatchmethod
from typing import TYPE_CHECKING, Any, ClassVar, Union

from bitcoinlib.hash_functions import sha256
from bitcoinlib.script import CScript

from .clause import (
    AbsoluteTimeSpec,
    Wait,
    CheckTemplateVerify,
    Clause,
    RevealPreImage,
    RelativeTimeSpec,
    SignedBy,
)
from .opcodes import AllowedOp
from .unassigned import PreImageVar, SignatureVar
from .witnessmanager import CTVHash, WitnessTemplate


class FragmentCompiler:
    """
    FragmentCompiler emits CScript fragments for DNFClauses. Fragments always
    leave the stack clean.  FragmentCompiler also populates data into the
    WitnessTemplate.
    """

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
    def _compile_signature(self, arg: SignedBy, witness: WitnessTemplate) -> CScript:
        if TYPE_CHECKING:
            assert callable(self._compile)
        witness.add(SignatureVar(arg))
        return CScript([arg.pubkey, AllowedOp.OP_CHECKSIGVERIFY])

    @_compile.register
    def _compile_preimage(
        self, arg: RevealPreImage, witness: WitnessTemplate
    ) -> CScript:
        if TYPE_CHECKING:
            assert callable(self._compile)
        witness.add(PreImageVar(arg))
        return CScript([AllowedOp.OP_SHA256, arg.image, AllowedOp.OP_EQUALVERIFY])

    @_compile.register
    def _compile_ctv(
        self, arg: CheckTemplateVerify, witness: WitnessTemplate
    ) -> CScript:
        witness.will_execute_ctv(CTVHash(arg.hash))
        return CScript([arg.hash, AllowedOp.OP_CHECKTEMPLATEVERIFY, AllowedOp.OP_DROP])

    @_compile.register
    def _compile_after(self, arg: Wait, witness: WitnessTemplate) -> CScript:
        # While valid to make this a witness variable, this is likely an error
        if isinstance(arg.time, AbsoluteTimeSpec):
            return CScript(
                [
                    arg.time.locktime,
                    AllowedOp.OP_CHECKLOCKTIMEVERIFY,
                    AllowedOp.OP_DROP,
                ]
            )
        if isinstance(arg.time, RelativeTimeSpec):
            return CScript(
                [
                    arg.time.sequence,
                    AllowedOp.OP_CHECKSEQUENCEVERIFY,
                    AllowedOp.OP_DROP,
                ]
            )
        raise ValueError(f"Unknown time type {arg.time!r}")
