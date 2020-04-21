from __future__ import annotations

import inspect
import typing
from typing import Any, Dict

from .bindable_contract import BindableContract
from .contract_base import ContractBase
from .decorators import PathFunction, UnlockFunction, PayAddress, CheckFunction


class MetaContract(type):

    def __new__(mcl, name, bases, nmspc):
        fields : Dict[str, Any] = typing.get_type_hints(nmspc['Fields'])
        nmspc['__annotations__'] = fields.copy()

        for base in bases:
            for method_name in dir(base):
                method = getattr(base, method_name)
                if hasattr(method, "__is_final_method__") and method.__is_final_method__:
                    if hasattr(method, "__call__"):
                        if method_name in nmspc:
                            raise ValueError("Cannot Override Final Method")
                    else:
                        raise ValueError("Cannot Override Final ???")

        nmspc['__slots__'] = bases[0].__slots__+ tuple(fields.keys())
        params = [inspect.Parameter("self", inspect.Parameter.POSITIONAL_ONLY)] + \
                 [inspect.Parameter(param,
                                    inspect.Parameter.KEYWORD_ONLY,
                                    annotation=type_)
                  for param, type_ in fields.items()]
        pay_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PayAddress)]
        path_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PathFunction)]
        unlock_funcs = [v for (k, v) in nmspc.items() if isinstance(v, UnlockFunction)]
        assertions = [v for (k, v) in nmspc.items() if isinstance(v, CheckFunction)]
        class MetaBase:
            init_class = ContractBase(fields, path_funcs, pay_funcs, unlock_funcs, assertions)
            def __init__(self, **kwargs: Dict[str, any]):
               self.init_class(self, **kwargs)
        return super(MetaContract, mcl).__new__(mcl, name, (MetaBase,)+bases, nmspc)


class Contract(BindableContract, metaclass=MetaContract):
    class Fields:
        pass

    class MetaData:
        color = lambda self: "brown"
        label = lambda self: "generic"

