from functools import singledispatchmethod
from typing import Any, Dict, List, NewType, Optional, Union

from bitcoinlib import segwit_addr
from bitcoinlib.address import script_to_p2wsh
from bitcoinlib.hash_functions import sha256
from bitcoinlib.script import CScript

from .opcodes import AllowedOp

CTVHash = NewType("CTVHash", bytes)


class WitnessTemplate:
    def __init__(self) -> None:
        self.witness: List[Union[CScript, int, bytes]] = []
        self.ctv_hash: Optional[CTVHash] = None

    def add(self, it: Union[CScript, int, bytes]) -> None:
        assert callable(self.internal_add)
        self.internal_add(it)

    @singledispatchmethod
    def internal_add(self, it: Union[CScript, bytes]) -> None:
        self.witness.insert(0, it)

    @internal_add.register
    def _(self, it: int) -> None:
        self.add(CScript([it]))

    def will_execute_ctv(self, ctv: CTVHash) -> None:
        if self.ctv_hash is not None and ctv != self.ctv_hash:
            raise AssertionError("Two CTV Hashes cannot be in the same witness")
        self.ctv_hash = ctv


class WitnessManager:
    def __init__(self) -> None:
        self.override_program: Optional[str] = None
        self.program: CScript = CScript()
        self.witnesses: Dict[int, WitnessTemplate] = {}
        self.is_final = False

    def to_json(self) -> Dict[str, Any]:
        assert self.is_final
        return {}

    def finalize(self) -> None:
        self.is_final = True

    def get_witness(self, key: int) -> List[Any]:
        assert self.is_final
        item = self.witnesses[key].witness.copy()
        item.append(self.program)
        return item

    def make_witness(self, key: int) -> WitnessTemplate:
        assert not self.is_final
        assert key not in self.witnesses
        self.witnesses[key] = WitnessTemplate()
        return self.witnesses[key]

    def get_p2wsh_script(self, main: bool=False) -> CScript:
        if self.override_program is not None:
            script = segwit_addr.decode("bc" if main else "bcrt", self.override_program)
            return CScript([script[0], bytes(script[1])])
        return CScript([AllowedOp.OP_0, sha256(self.program)])

    def get_p2wsh_address(self) -> str:
        if self.override_program is not None:
            return self.override_program
        return script_to_p2wsh(self.program)
