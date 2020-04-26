from typing import NewType, Dict, Any, List

from sapio.bitcoinlib import segwit_addr
from sapio.bitcoinlib.address import script_to_p2wsh
from sapio.bitcoinlib.hash_functions import sha256
from sapio.bitcoinlib.script import CScript
from sapio.script.opcodes import AllowedOp
from sapio.util import methdispatch
from typing import Optional, Union
from functools import singledispatchmethod

CTVHash = NewType("CTVHash", bytes)


class WitnessTemplate:
    def __init__(self):
        self.witness = []
        self.ctv_hash : Optional[CTVHash] = None
    def add(self, it: Union[CScript, int, bytes]) -> None:
        assert callable(self.internal_add)
        self.internal_add(it)
    @singledispatchmethod
    def internal_add(self, it : Union[CScript, bytes]):
        self.witness.insert(0, it)
    @internal_add.register
    def _(self, it: int):
        self.add(CScript([it]))
    def will_execute_ctv(self, ctv:CTVHash):
        if self.ctv_hash is not None and ctv != self.ctv_hash:
            raise AssertionError("Two CTV Hashes cannot be in the same witness")
        self.ctv_hash = ctv


class WitnessManager:
    def __init__(self) -> None:
        self.override_program: Optional[str] = None
        self.program: CScript = CScript()
        self.witnesses : Dict[Any, WitnessTemplate] = {}
        self.is_final = False
    def to_json(self):
        assert self.is_final

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