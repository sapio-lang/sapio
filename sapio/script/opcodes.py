from __future__ import annotations
from typing import Union

from numpy import uint32

from sapio.bitcoinlib.script import *

class AllowedOp:
    # Control Flow
    OP_IF = OP_IF
    OP_NOTIF = OP_NOTIF
    OP_ELSE = OP_ELSE
    OP_ENDIF = OP_ENDIF
    # Basic
    OP_0 = OP_0
    OP_1 = OP_1
    # Math
    OP_1SUB = OP_1SUB
    OP_WITHIN = OP_WITHIN
    # Stack
    OP_DUP = OP_DUP
    OP_IFDUP = OP_IFDUP
    OP_DROP = OP_DROP
    # Crypto
    OP_SHA256 = OP_SHA256
    # Context Verification
    OP_CHECKSIGVERIFY = OP_CHECKSIGVERIFY
    OP_CHECKTEMPLATEVERIFY = OP_CHECKTEMPLATEVERIFY
    OP_CHECKLOCKTIMEVERIFY = OP_CHECKLOCKTIMEVERIFY
    OP_CHECKSEQUENCEVERIFY = OP_CHECKSEQUENCEVERIFY
    OP_VERIFY = OP_VERIFY
    OP_EQUALVERIFY = OP_EQUALVERIFY

ONE = CScriptNum(1)
ZERO = CScriptNum(0)
ONE_enc = CScriptNum.encode(ONE)
ZERO_enc = CScriptNum.encode(ZERO)
def bool_of_stack_item(v:bytes):
    for idx, char in enumerate(v):
        if char != 0:
            if idx == len(v) -1 and char == 0x80:
                return False
            return True
    return False


# Interpreter Loosely Based on Bitcoin Core, MIT License
def handle(op:Union[CScriptOp, int, bytes], stack: List[bytes], handle_branch: ConditionStack):
    should_execute = handle_branch.all_true()
    if isinstance(op, bytes):
        if should_execute:
            stack.append(op)
    if isinstance(op, int):
        if should_execute:
            stack.append(CScriptNum.encode(CScriptNum(op)))
    if isinstance(op, CScriptOp):
        if False: pass
        # Control Flow
        elif op == OP_IF or op == OP_NOTIF:
            branch_value = False
            if should_execute:
                if len(stack) < 1: return False
                cond = stack.pop()
                # TODO: These Rules are only for Segwit + MinimalIF
                if len(cond) > 1:
                    return False
                if len(cond) == 1 and cond[0] != 1:
                    return False
                branch_value = bool_of_stack_item(cond)
                if op == OP_NOTIF:
                    branch_value = not branch_value
            handle_branch.push_back(branch_value)

        elif op == OP_ELSE:
            if handle_branch.empty():
                return False
            handle_branch.toggle_top()
        elif op == OP_ENDIF:
            if handle_branch.empty():
                return False
            handle_branch.pop_back()
        elif not should_execute:
            return True
        # Crypto
        elif op == OP_SHA256:
            if len(stack) < 1: return False
        # Math
        elif op == OP_1SUB:
            if len(stack) < 1: return False
            num = CScriptNum.decode(stack.pop()) - ONE
            stack.append(num)
        elif op == OP_WITHIN:
            if len(stack) < 3: return False
            upper = CScriptNum.decode(stack.pop())
            lower = CScriptNum.decode(stack.pop())
            num = CScriptNum.decode(stack.pop())
            ret = lower <= num < upper
            stack.append(bytes(1) if ret else bytes(0))
        # Basic
        elif op == OP_0:
            stack.append(ZERO_enc)
        elif op == OP_1:
            stack.append(ONE_enc)
        # Stack
        elif op == OP_DROP:
            if len(stack) < 1: return False
            stack.pop()
        elif op == OP_IFDUP:
            if len(stack) < 1: return False
            v = stack[-1]
            b = bool_of_stack_item(v)
            if b:
                # TODO: Is Copy needed?
                stack.append(v)

        elif op == OP_DUP:
            if len(stack) < 1: return False
            stack.append(stack[-1])
        # Verification
        # TODO: Validation assumed true,
        # This just handles the stack operations
        elif op == OP_CHECKSIGVERIFY:
            if len(stack) < 2: return False
            key = stack.pop()
            sig = stack.pop()
        elif op == OP_CHECKTEMPLATEVERIFY:
            if len(stack) < 1: return False
        elif op == OP_CHECKLOCKTIMEVERIFY:
            if len(stack) < 1: return False
        elif op == OP_CHECKSEQUENCEVERIFY:
            if len(stack) < 1: return False
        elif op == OP_EQUALVERIFY:
            if len(stack) < 2: return False
            a = stack.pop()
            b = stack.pop()
            if a != b:
                return False
        elif op == OP_VERIFY:
            if len(stack) < 1: return False
            b = stack.pop()
            if not bool_of_stack_item(b):
                return False
        else:
            raise ValueError("Scripts using ", op, " cannot be interpreted yet!")
        return True

# Implementation Based on Bitcoin Core Condition Stack, MIT License
class ConditionStack:
    NO_FALSE = iinfo(uint32).max
    def __init__(self):
        self.stack_size = 0
        self.first_false_position = ConditionStack.NO_FALSE
    def empty(self):
        return self.stack_size == 0
    def all_true(self):
        return self.first_false_position == ConditionStack.NO_FALSE
    def push_back(self, b:bool):
        if self.first_false_position == ConditionStack.NO_FALSE and not b:
            self.first_false_position = self.stack_size
        self.stack_size +=1
    def pop_back(self):
        assert self.stack_size > 0
        self.stack_size -= 1
        if self.first_false_position == self.stack_size:
            self.first_false_position = ConditionStack.NO_FALSE
    def toggle_top(self):
        assert self.stack_size > 0
        if self.first_false_position == ConditionStack.NO_FALSE:
            self.first_false_position = self.stack_size - 1
        elif self.first_false_position == self.stack_size -1:
            self.first_false_position = ConditionStack.NO_FALSE
        else:
            pass


def interpret(s:CScript) -> bool:
    stack : List[bytes] = []
    handle_branch = ConditionStack()
    for op in iter(s):
        if not handle(op, stack, handle_branch):
            return False
    return True
