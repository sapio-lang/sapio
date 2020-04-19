from typing import List

from sapio.script.clause import Clause, AndClause, OrClause, SignatureCheckClause, \
    PreImageCheckClause, CheckTemplateVerifyClause, AfterClause, SatisfiedClause, DNFClause, DNF
from sapio.util import methdispatch


# Assumes that there is no OR which comes after an AND

class FlattenPass:
    @methdispatch
    def flatten(self, arg: Clause, or_allowed: bool=True) -> DNF:
        raise NotImplementedError("Cannot Compile Arg", arg)


    @flatten.register
    def flatten_and(self, arg: AndClause, or_allowed=False) -> DNF:
        l = self.flatten(arg.a, or_allowed)
        l2 = self.flatten(arg.b, or_allowed)
        assert len(l) == 1
        assert len(l2) == 1
        l[0].extend(l2[0])
        return l


    @flatten.register
    def flatten_sat(self, arg:SatisfiedClause, or_allowed=False) -> DNF:
        return [[]]

    @flatten.register
    def flatten_or(self, arg: OrClause, or_allowed=True) -> DNF:
        assert or_allowed
        return self.flatten(arg.a, or_allowed) + self.flatten(arg.b, or_allowed)

    @flatten.register(AfterClause)
    @flatten.register(CheckTemplateVerifyClause)
    @flatten.register(PreImageCheckClause)
    @flatten.register(SignatureCheckClause)
    def flatten_after(self, arg, or_allowed=False) -> DNF:
        return [[arg]]

try:
    FlattenPass().flatten(AndClause(OrClause(SatisfiedClause(), SatisfiedClause()), OrClause(SatisfiedClause(), SatisfiedClause())))
    raise AssertionError("this sanity check should fail")
except AssertionError:
    pass
