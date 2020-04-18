from functools import reduce

from sapio.bitcoinlib.static_types import PubKey, Amount
from sapio.contract import Contract, unlock, path, TransactionTemplate
from sapio.script.clause import SignatureCheckClause, Variable, List, Bitcoin

from itertools import combinations

def multisig(l, n):
    assert len(l) > n
    assert n > 0
    l2 = [SignatureCheckClause(Variable("key_"+str(i), v)) for i,v in enumerate(l)]
    l3 = [reduce(lambda a, b: a * b, combo[1:], combo[0])
          for combo in combinations(l2, n)]
    return reduce(lambda  a,b: a+b, l3[1:], l3[0])




# Demonstrates multisig without using any special multisig functionality
class RawMultiSig(Contract):
    class Fields:
        keys: List[PubKey]
        thresh: int
    @unlock(lambda self: multisig(self.keys.assigned_value, self.thresh.assigned_value))
    def _(self): pass

# Demonstrates multisig with a default path accessible at a lower threshold
class RawMultiSigWithPath(Contract):
    class Fields:
        keys: List[PubKey]
        thresh_all: int
        thresh_path: int
        path: Contract
        amount : Amount

    @unlock(lambda self: multisig(self.keys.assigned_value, self.thresh_all.assigned_value))
    def _(self): pass

    @path(lambda self: multisig(self.keys.assigned_value, self.thresh_path.assigned_value))
    def redeem(self):
        tx = TransactionTemplate()
        tx.add_output(self.amount.assigned_value, self.path.assigned_value)
        return tx
