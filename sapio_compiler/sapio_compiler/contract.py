from __future__ import annotations

import inspect
import typing
from typing import Any, Dict

from .core.bindable_contract import BindableContract
from .core.contract_base import ContractBase
from .decorators import CheckFunction, PathFunction, PayAddress, UnlockFunction


class MetaContract(type):
    """
    MetaContract is a base metaclass which handles the creation of a
    new Contract instance and stitches the relevant parts together into a
    class that can be initialized correctly.

    It should not be inherited from directly, prefer to inherit from
    Contract which inherits from BindableContract.
    """
    def __new__(mcl, name, bases, nmspc):
        pay_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PayAddress)]
        path_funcs = [v for (k, v) in nmspc.items() if isinstance(v, PathFunction)]
        unlock_funcs = [v for (k, v) in nmspc.items() if isinstance(v, UnlockFunction)]
        assertions = [v for (k, v) in nmspc.items() if isinstance(v, CheckFunction)]

        class MetaBase(BindableContract[nmspc["Fields"]]):
            """MetaBase is the actual class which gets constructed"""
            init_class = ContractBase(
                nmspc["Fields"], path_funcs, pay_funcs, unlock_funcs, assertions
            )

        return super(MetaContract, mcl).__new__(mcl, name, (MetaBase,), nmspc)


class Contract(BindableContract, metaclass=MetaContract):
    """Base class to inherit from when making a new contract"""
    class Fields:
        """
        Mock-value for subcontract to replace.

        Fields should be just a type list with no values

        Examples
        --------
        >>> class Fields:
        ...     amount: Amount
        ...     steps: int

        """
