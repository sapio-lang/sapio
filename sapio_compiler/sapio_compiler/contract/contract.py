from __future__ import annotations

import inspect
import typing
from typing import Any, Dict

from .core.bindable_contract import BindableContract
from .core.contract_base import ContractBase
from .decorators import PathFunction, UnlockFunction, PayAddress, CheckFunction


class MetaContract(type):

    def __new__(mcl, name, bases, nmspc):
        pay_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PayAddress)]
        path_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PathFunction)]
        unlock_funcs = [v for (k, v) in nmspc.items() if isinstance(v, UnlockFunction)]
        assertions = [v for (k, v) in nmspc.items() if isinstance(v, CheckFunction)]

        class MetaBase(BindableContract[nmspc['Fields']]):
            init_class = ContractBase(nmspc['Fields'], path_funcs, pay_funcs, unlock_funcs, assertions)

        return super(MetaContract, mcl).__new__(mcl, name, (MetaBase,), nmspc)


class Contract(BindableContract, metaclass=MetaContract):
    class Fields:
        pass
