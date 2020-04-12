from __future__ import annotations
from util import *
from typing import List
from typing import TypeVar, Generic, Any
from typing import Union
from bitcoinlib.script import CScript

from opcodes import Op, PushNumber
from my_types import *

MODE = "+"  # "str"


class StandardClauseMixin:
    def __add__(self: Clause, other: Clause) -> Clause:
        return OrClause(self, other)

    def __mul__(self: Clause, other: Clause) -> Clause:
        return AndClause(self, other)

    def __str__(self) -> str:
        if MODE == "+":
            if self.__class__.n_args == 1:
                return "{}({})".format(self.__class__.__name__, self.a)
            elif self.__class__.n_args == 2:
                return "{}{}{}".format(self.a, self.symbol, self.b)
            else:
                return "{}()".format(self.__class__.__name__)
        if self.__class__.n_args == 1:
            return "{}({})".format(self.__class__.__name__, self.a)
        elif self.__class__.n_args == 2:
            return "{}({}, {})".format(self.__class__.__name__, self.a, self.b)
        else:
            return "{}()".format(self.__class__.__name__)


class SatisfiedClause(StandardClauseMixin):
    n_args = 0
class UnsatisfiableClause(StandardClauseMixin):
    n_args = 0


class AndClause(StandardClauseMixin):
    n_args = 2
    symbol = "*"

    def __init__(self, a: Clause, b: Clause):
        self.a = a
        self.b = b


class OrClause(StandardClauseMixin):
    n_args = 2
    symbol = "+"

    def __init__(self, a: Clause, b: Clause):
        self.a = a
        self.b = b


class SignatureCheckClause(StandardClauseMixin):
    n_args = 1
    def __init__(self, a: Variable[PubKey]):
        self.a = a
        self.b = a.sub_variable("signature")


class PreImageCheckClause(StandardClauseMixin):
    n_args = 1

    def __init__(self, a: Variable[Hash]):
        self.a = a
        self.b = a.sub_variable("preimage")


class CheckTemplateVerifyClause(StandardClauseMixin):
    n_args = 1

    def __init__(self, a: Variable[Hash]):
        self.a = a

    @staticmethod
    def make(name: str, outputs: List[(int, Clause)], n_inputs=1, input_index=0, sequences=(0,), lock_time: int = 0):
        # TODO: Return the actual Hash here
        a: Variable[Hash] = Variable(name, Hash(b" <h(x)>"))
        return CheckTemplateVerifyClause(a)


class AbsoluteTimeSpec: pass


class RelativeTimeSpec:
    def __init__(self, t):
        self.time = t


TimeSpec = Union[AbsoluteTimeSpec, RelativeTimeSpec]

def Weeks(n):
    return Variable("RelativeTimeSpec({} Weeks)".format(n), RelativeTimeSpec(n))


class AfterClause(StandardClauseMixin):
    n_args = 1

    def __init__(self, a: Variable[TimeSpec]):
        self.a = a


V = TypeVar('V')


class Variable(Generic[V]):
    def __init__(self, name: str, value: Optional[V] = None):
        self.name: str = name
        self.value: Optional[V] = value
        self.sub_variable_count = -1

    def sub_variable(self, purpose: str, value: Optional[V] = None) -> Variable:
        self.sub_variable_count += 1
        return Variable(self.name + "_" + str(self.sub_variable_count) + "_" + purpose, value)

    def assign(self, value: V):
        self.value = value

    def __str__(self):
        return "{}('{}', {})".format(self.__class__.__name__, self.name, self.value)


Clause: object = Union[UnsatisfiableClause, SatisfiedClause,
               Variable,
               OrClause,
               AndClause,
               SignatureCheckClause,
               PreImageCheckClause,
               CheckTemplateVerifyClause,
               AfterClause]

T = TypeVar('T')


class ProgramBuilder:

    def bind(self, variable: Variable[T], value: T):
        pass

    def compile_cnf(self, clause: Clause) -> List[List[Union[
        SatisfiedClause, Variable, OrClause, AndClause, SignatureCheckClause, PreImageCheckClause, CheckTemplateVerifyClause, AfterClause]]]:
        # TODO: Figure out how many passes are required / abort when stable
        # 1000 should be enough that covers all valid scripts...
        for x in range(1000):
            clause = self.normalize(clause)
        return self.flatten(clause)

    def compile(self, clause: Clause) -> (bytes, List[Any]):
        cnf = self.compile_cnf(clause)
        cases = len(cnf)
        witnesses = [[]] * len(cnf)
        script = b""
        # If we have one or two cases, special case the emitted scripts
        # 3 or more, use a generic wrapper
        if cases == 1:
            for frag in cnf[0]:
                script += self._compile(frag, witnesses[0])
                # Hack because the fragment compiler leaves stack empty
                script += PushNumber(1)
        if cases == 2:
            witnesses[0] = [1]
            witnesses[1] = [0]
            # note order of side effects!
            branch_a = CScript([self._compile(frag, witnesses[0]) for frag in cnf[0]])
            branch_b = CScript([self._compile(frag, witnesses[1]) for frag in cnf[1]])
            script = CScript([Op.If,
                               branch_a,
                               Op.Else,
                               branch_b,
                               Op.EndIf,
                               1])
        else:
            # Check that the first argument passed is an in range execution path
            script = CScript([Op.Dup, 0, cases, Op.Within, Op.Verify])
            for (idx, frag) in enumerate(cnf):
                witnesses[idx] = [idx + 1]
                script += Op.SubOne + Op.IfDup + Op.NotIf

                for cl in frag:
                    script += self._compile(cl, witnesses[idx])
                script += Op.Zero + Op.EndIf
        return script, witnesses

    # Normalize Bubbles up all the OR clauses into a CNF
    @methdispatch
    def normalize(self, arg: Clause) -> Clause:
        raise NotImplementedError("Cannot Compile Arg")

    @normalize.register
    def _(self, arg: AndClause) -> Clause:
        class_key = (arg.a.__class__, arg.b.__class__)
        try:
            return self.normalize({
                                      # Swap values to go to other case
                                      (OrClause, AndClause): lambda: AndClause(arg.b, arg.a),
                                      (AndClause, OrClause): lambda: OrClause(AndClause(arg.a, arg.b.a),
                                                                      AndClause(arg.a, arg.b.b)),
                                      (OrClause, OrClause): lambda: OrClause(
                                          OrClause(AndClause(arg.a.a, arg.b.a), AndClause(arg.a.a, arg.b.b)),
                                          OrClause(AndClause(arg.a.b, arg.b.a), AndClause(arg.a.b, arg.b.b))),
                                  }[class_key])()
        except KeyError:
            if isinstance(arg.a, AndClause):
                return AndClause(self.normalize(arg.a), arg.b)
            if isinstance(arg.a, OrClause):
                return OrClause(AndClause(arg.a.a, arg.b), AndClause(arg.a.b, arg.b))
            if isinstance(arg.b, AndClause):
                return AndClause(self.normalize(arg.b), arg.a)
            if isinstance(arg.b, OrClause):
                return OrClause(AndClause(arg.b.a, arg.a), AndClause(arg.b.b, arg.a))
            return arg

    @normalize.register
    def _(self, arg: OrClause) -> Clause:
        return OrClause(self.normalize(arg.a), self.normalize(arg.b))

    # TODO: Unionize!

    @normalize.register
    def _(self, arg: SignatureCheckClause) -> Clause:
        return arg

    @normalize.register
    def _(self, arg: PreImageCheckClause) -> Clause:
        return arg

    @normalize.register
    def _(self, arg: CheckTemplateVerifyClause) -> Clause:
        return arg

    @normalize.register
    def _(self, arg: AfterClause) -> Clause:
        return arg

    @normalize.register
    def _(self, arg: Variable) -> Clause:
        return arg

    @methdispatch
    def flatten(self, arg: Clause) -> List[List[Clause]]:
        raise NotImplementedError("Cannot Compile Arg")

    @flatten.register
    def _(self, arg: AndClause) -> List[List[Clause]]:
        assert not isinstance(arg.a, OrClause)
        assert not isinstance(arg.b, OrClause)
        l = self.flatten(arg.a)
        l2 = self.flatten(arg.b)
        assert len(l) == 1
        assert len(l2) == 1
        l[0].extend(l2[0])
        return l

    @flatten.register
    def _(self, arg: OrClause) -> List[List[Clause]]:
        return self.flatten(arg.a) + self.flatten(arg.b)

    @flatten.register
    def _(self, arg: SignatureCheckClause) -> List[List[Clause]]:
        return [[arg]]

    @flatten.register
    def _(self, arg: PreImageCheckClause) -> List[List[Clause]]:
        return [[arg]]

    @flatten.register
    def _(self, arg: CheckTemplateVerifyClause) -> List[List[Clause]]:
        return [[arg]]

    @flatten.register
    def _(self, arg: AfterClause) -> List[List[Clause]]:
        return [[arg]]

    @flatten.register
    def _(self, arg: Variable) -> List[List[Clause]]:
        return [[arg]]

    @methdispatch
    def _compile(self, arg: Clause, witness) -> CScript:
        raise NotImplementedError("Cannot Compile Arg", arg)

    @_compile.register
    def _(self, arg: SignatureCheckClause, witness) -> CScript:
        return self._compile(arg.b, witness) + self._compile(arg.a, witness) + CScript([Op.Check_sig_verify])

    @_compile.register
    def _(self, arg: PreImageCheckClause, witness) -> CScript:
        return self._compile(arg.b, witness) + Op.Sha256 + self._compile(arg.a, witness) + Op.Equal

    @_compile.register
    def _(self, arg: CheckTemplateVerifyClause, witness) -> CScript:
        # While valid to make this a witness variable, this is likely an error
        print(arg, arg.a, arg.a.value)
        assert arg.a.value is not None
        return self._compile(arg.a, witness) + CScript([Op.CheckTemplateVerify, Op.Drop])

    @_compile.register
    def _(self, arg: AfterClause, witness) -> CScript:
        # While valid to make this a witness variable, this is likely an error
        assert arg.a.value is not None
        if isinstance(arg.a.value, AbsoluteTimeSpec):
            return CScript([arg.a.value.time, Op.CheckLockTimeVerify, Op.Drop])
        if isinstance(arg.a.value, RelativeTimeSpec):
            return CScript([arg.a.value.time, Op.CheckSequenceVerify, Op.Drop])
        raise ValueError

    @_compile.register
    def _(self, arg: Variable, witness) -> CScript:
        if arg.value is None:
            # Todo: this is inefficient...
            witness.insert(0, arg.name)
            return b""
        else:
            return arg.value
