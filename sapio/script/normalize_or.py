from sapio.script.clause import Clause, AndClause, AndClauseArgument, OrClause, SignatureCheckClause, \
    PreImageCheckClause, CheckTemplateVerifyClause, AfterClause, Variable, UnsatisfiableClause
from sapio.util import methdispatch


class NormalizationPass:
    def __init__(self):
        self.took_action: bool = False
    # Normalize Bubbles up all the OR clauses into a CNF
    @methdispatch
    def normalize(self, arg: Clause) -> AndClauseArgument:
        raise NotImplementedError("Cannot Compile Arg", arg)

    @normalize.register
    def normalize_and(self, arg: AndClause) -> AndClauseArgument:
        a: AndClauseArgument = arg.a
        b: AndClauseArgument = arg.b
        ret : Clause = arg
        if isinstance(a, OrClause) and isinstance(b, OrClause):
            self.took_action = True
            a0: AndClauseArgument = self.normalize(a.a)
            a1: AndClauseArgument = self.normalize(a.b)
            b0: AndClauseArgument = self.normalize(b.a)
            b1: AndClauseArgument = self.normalize(b.b)
            ret = (a0 & b0) | (a0 & b1) | (a1 & b0) | (a1 & b1)
        elif isinstance(b, AndClause) and isinstance(a, OrClause):
            self.took_action = True
            _or, _and = self.normalize(a), self.normalize(b)
            ret = (_and & _or.a) | (_and & _or.b)
        elif isinstance(a, AndClause) and isinstance(b, OrClause):
            self.took_action = True
            _or, _and = self.normalize(b), self.normalize(a)
            ret =(_and & _or.a) | (_and & _or.b)
        # Other Clause can be ignored...
        elif isinstance(a, AndClause):
            ret = self.normalize(a)&b
        elif isinstance(a, OrClause):
            self.took_action = True
            a0, a1 = self.normalize(a.a), self.normalize(a.b)
            ret = (a0 & b) | (a1 & b)
        elif isinstance(b, AndClause):
            ret = self.normalize(b)&a
        elif isinstance(b, OrClause):
            self.took_action = True
            b0, b1 = self.normalize(b.a), self.normalize(b.b)
            ret = (b0 & a) | (b1 & a)
        return ret

    @normalize.register
    def normalize_or(self, arg: OrClause) -> AndClauseArgument:
        # switching order guarantees that successive passes will
        # have a chance to unwind unsatisfiable clauses
        return self.normalize(arg.b) | self.normalize(arg.a)

    @normalize.register(UnsatisfiableClause)
    @normalize.register(SignatureCheckClause)
    @normalize.register(PreImageCheckClause)
    @normalize.register(CheckTemplateVerifyClause)
    @normalize.register(AfterClause)
    def normalize_unsat(self, arg: UnsatisfiableClause) -> Clause:
        return arg

