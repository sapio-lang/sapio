
from .clause import SignatureCheckClause, PreImageCheckClause



class Variable:
    pass

class SignatureVar(Variable):
    def __init__(self, pk: SignatureCheckClause) -> None:
        self.pk = pk.a.assigned_value


class PreImageVar(Variable):
    def __init__(self, image: PreImageCheckClause) -> None:
        self.image = image.a.assigned_value