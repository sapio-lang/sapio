from typing import List

from sapio.script.clause import Clause, AndClause, OrClause, SignatureCheckClause, \
    PreImageCheckClause, CheckTemplateVerifyClause, AfterClause, Variable, SatisfiedClause
from sapio.util import methdispatch


# Assumes that there is no OR which comes after an AND

class FlattenPass:
    @methdispatch
    def flatten(self, arg: Clause, or_allowed: bool=True) -> List[List[Clause]]:
        raise NotImplementedError("Cannot Compile Arg", arg)


    @flatten.register
    def flatten_and(self, arg: AndClause, or_allowed=False) -> List[List[Clause]]:
        l = self.flatten(arg.a, or_allowed)
        l2 = self.flatten(arg.b, or_allowed)
        assert len(l) == 1
        assert len(l2) == 1
        l[0].extend(l2[0])
        return l


    @flatten.register
    def flatten_sat(self, arg:SatisfiedClause, or_allowed=False):
        return [[]]

    @flatten.register
    def flatten_or(self, arg: OrClause, or_allowed=True) -> List[List[Clause]]:
        assert or_allowed
        return self.flatten(arg.a, or_allowed) + self.flatten(arg.b, or_allowed)


    @flatten.register
    def flatten_sigcheck(self, arg: SignatureCheckClause, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]


    @flatten.register
    def flatten_preimage(self, arg: PreImageCheckClause, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]


    @flatten.register
    def flatten_ctv(self, arg: CheckTemplateVerifyClause, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]


    @flatten.register
    def flatten_after(self, arg: AfterClause, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]


    @flatten.register
    def flatten_var(self, arg: Variable, or_allowed=False) -> List[List[Clause]]:
        return [[arg]]

try:
    FlattenPass().flatten(AndClause(OrClause(SatisfiedClause(), SatisfiedClause()), OrClause(SatisfiedClause(), SatisfiedClause())))
    raise AssertionError("this sanity check should fail")
except AssertionError:
    pass
