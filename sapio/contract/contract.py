from __future__ import annotations

import inspect
import typing
from typing import Any, Dict

from .bindable_contract import BindableContract
from .contract_base import ContractBase
from .decorators import PathFunction, UnlockFunction, PayAddress, CheckFunction, HasFinal


class MetaContract(HasFinal):

    def __new__(mcl, name, bases, nmspc):
        fields: Dict[str, Any] = typing.get_type_hints(nmspc['Fields'])

        pay_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PayAddress)]
        path_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PathFunction)]
        unlock_funcs = [v for (k, v) in nmspc.items() if isinstance(v, UnlockFunction)]
        assertions = [v for (k, v) in nmspc.items() if isinstance(v, CheckFunction)]

        class MetaBase(BindableContract):
            init_class = ContractBase(fields, path_funcs, pay_funcs, unlock_funcs, assertions)
            __slots__ = BindableContract.__slots__ + tuple(fields.keys())
            __annotations__ = fields

            def __init__(self, **kwargs: Dict[str, any]):
                self.init_class(self, **kwargs)
        return super(MetaContract, mcl).__new__(mcl, name, (MetaBase,), nmspc)


class Contract(metaclass=MetaContract):
    class Fields:
        pass
