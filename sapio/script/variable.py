from __future__ import annotations
from typing import TypeVar, Generic, Union, Optional

V = TypeVar('V')

class UnassignedVariable(Generic[V]):
    def __init__(self, name: Union[bytes, str]):
        self.name: bytes = bytes(name, 'utf-8') if isinstance(name, str) else name

# The type V must be something that can be put onto the stack...
class AssignedVariable(Generic[V]):
    def __init__(self, name: Union[bytes, str], value: V):
        self.name: bytes = bytes(name, 'utf-8') if isinstance(name, str) else name
        self.assigned_value: V = value
        self.sub_variable_count = -1

    def sub_variable(self, purpose: str) -> UnassignedVariable:
        self.sub_variable_count += 1
        return UnassignedVariable(self.name + b"_" + bytes(str(self.sub_variable_count), 'utf-8') + b"_" + bytes(purpose, 'utf-8'))

    def __str__(self):
        return "{}('{}', {})".format(self.__class__.__name__, self.name, self.assigned_value)

Variable = Union[UnassignedVariable, AssignedVariable]