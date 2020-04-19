from __future__ import annotations
from typing import TypeVar, Generic, Union, Optional

V = TypeVar('V')


class Variable(Generic[V]):
    def __init__(self, name: Union[bytes, str], value: Optional[V] = None):
        self.name: bytes = bytes(name, 'utf-8') if isinstance(name, str) else name
        self.assigned_value: Optional[V] = value
        self.sub_variable_count = -1

    def sub_variable(self, purpose: str, value: Optional[V] = None) -> Variable:
        self.sub_variable_count += 1
        return Variable(self.name + b"_" + bytes(str(self.sub_variable_count), 'utf-8') + b"_" + bytes(purpose, 'utf-8'), value)

    def assign(self, value: V):
        self.assigned_value = value

    def __str__(self):
        return "{}('{}', {})".format(self.__class__.__name__, self.name, self.assigned_value)