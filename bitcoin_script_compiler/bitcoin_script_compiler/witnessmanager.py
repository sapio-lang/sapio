from functools import singledispatchmethod
from typing import Any, ClassVar, Dict, List, NewType, Optional, Union, TYPE_CHECKING

from sapio_bitcoinlib import segwit_addr
from sapio_bitcoinlib.address import script_to_p2wsh
from sapio_bitcoinlib.hash_functions import sha256
from sapio_bitcoinlib.script import CScript

from .opcodes import AllowedOp
from .unassigned import PreImageVar, SignatureVar, Variable

CTVHash = NewType("CTVHash", bytes)


class MultipleCTVError(Exception):
    pass


class WitnessTemplate:
    """
    A WitnessTemplate contains all the information needed to be able to sign/generate
    a spend for a specific pathway through a script.
    """

    pending: Dict[int, Variable]
    """Mapping from witness stack position to the data to fill"""
    witness: List[bytes]
    """The stack to pass to the witness for this input. If pending variables,
    pending.keys() are dummy variables"""
    ctv_hash: Optional[CTVHash]
    """The ctv hash that will be required for the tx to be valid. Useful for
    linking this to another transaction
    """

    def __init__(self) -> None:
        self.pending: Dict[int, Variable] = {}
        self.witness: List[bytes] = []
        self.ctv_hash: Optional[CTVHash] = None

    def add(self, it: Union[CScript, int, bytes, Variable]) -> None:
        if TYPE_CHECKING:
            assert callable(self.internal_add)
        self.internal_add(it)

    @singledispatchmethod
    def internal_add(self, it: Union[CScript, bytes]) -> None:
        self.witness.insert(0, it)

    PREFIX: ClassVar[bytes] = sha256(bytes(1000))

    @internal_add.register
    def _add_sig(self, it: SignatureVar) -> None:
        idx = len(self.witness)
        self.pending[idx] = it
        self.add(self.PREFIX + b"_sig_by_" + it.pk.get_bytes())

    @internal_add.register
    def _add_preim(self, it: PreImageVar) -> None:
        idx = len(self.witness)
        self.pending[idx] = it
        self.add(self.PREFIX + b"_preim_of_" + it.image)

    @internal_add.register
    def _add_int(self, it: int) -> None:
        self.add(CScript([it]))

    def will_execute_ctv(self, ctv: CTVHash) -> None:
        if self.ctv_hash is not None and ctv != self.ctv_hash:
            raise MultipleCTVError("Two CTV Hashes cannot be in the same witness")
        self.ctv_hash = ctv


class FinalizationNotComplete(Exception):
    pass


class FinalizationComplete(Exception):
    pass


from sapio_bitcoinlib import miniscript


class WitnessManager:
    def __init__(self, ms: miniscript.Node) -> None:
        self.override_program: Optional[str] = None
        self.ms = ms
        try:
            self._program = ms.script
        except:
            pass

    @property
    def program(self):
        if self.override_program is None:
            return self._program
        else:
            return CScript()

    def to_json(self) -> Dict[str, Any]:
        return {}

    def get_p2wsh_script(self, main: bool = False) -> CScript:
        if self.override_program is not None:
            (version, program) = segwit_addr.decode(
                "bc" if main else "bcrt", self.override_program
            )
            if version is None or program is None:
                raise ValueError("Corrupt override program")
            return CScript([version, bytes(program)])
        return CScript([AllowedOp.OP_0, sha256(self.ms.script)])

    def get_p2wsh_address(self) -> str:
        if self.override_program is not None:
            return self.override_program
        return script_to_p2wsh(self.ms.script)
