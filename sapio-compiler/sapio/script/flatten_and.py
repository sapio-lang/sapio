from typing import List, TYPE_CHECKING, Union

from sapio.script.clause import (
    Clause,
    AndClause,
    OrClause,
    SignatureCheckClause,
    PreImageCheckClause,
    CheckTemplateVerifyClause,
    AfterClause,
    SatisfiedClause,
    DNFClause,
    DNF,
)
from functools import singledispatchmethod


# Assumes that there is no OR which comes after an AND


class FlattenPass:
    def __call__(self, arg:Clause, or_allowed: bool=True) -> DNF:
        if TYPE_CHECKING:
            assert callable(self.flatten)
        return self.flatten(arg, or_allowed)
        
    @singledispatchmethod
    def flatten(self, arg: Clause, or_allowed: bool = True) -> DNF:
        raise NotImplementedError("Cannot Compile Arg", arg)

    @flatten.register
    def flatten_and(self, arg: AndClause, or_allowed: bool = False) -> DNF:
        if TYPE_CHECKING:
            assert callable(self.flatten)
        l: DNF = self.flatten(arg.a, or_allowed)
        l2: DNF = self.flatten(arg.b, or_allowed)
        assert len(l) == 1
        assert len(l2) == 1
        l[0].extend(l2[0])
        return l

    @flatten.register
    def flatten_sat(self, arg: SatisfiedClause, or_allowed: bool = False) -> DNF:
        return [[]]

    @flatten.register
    def flatten_or(self, arg: OrClause, or_allowed: bool = True) -> DNF:
        if TYPE_CHECKING:
            assert callable(self.flatten)
        assert or_allowed
        l : DNF =  self.flatten(arg.a, or_allowed)
        l2: DNF =  self.flatten(arg.b, or_allowed)
        return l+l2


    @flatten.register(AfterClause)
    @flatten.register(CheckTemplateVerifyClause)
    @flatten.register(PreImageCheckClause)
    @flatten.register(SignatureCheckClause)
    def flatten_after(
        self,
        arg: Union[
            AfterClause,
            CheckTemplateVerifyClause,
            PreImageCheckClause,
            SignatureCheckClause,
        ],
        or_allowed: bool = False,
    ) -> DNF:
        return [[arg]]


# TODO: Move to a test...
try:
    f = FlattenPass()
    assert callable(f.flatten)
    f.flatten(
        AndClause(
            OrClause(SatisfiedClause(), SatisfiedClause()),
            OrClause(SatisfiedClause(), SatisfiedClause()),
        )
    )
    raise AssertionError("this sanity check should fail")
except AssertionError:
    pass
