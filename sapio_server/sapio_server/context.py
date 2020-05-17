from sapio_compiler import BindableContract
from typing import Dict, Optional


class Context:
    compilation_cache: Dict[str, BindableContract]

    def __init__(self):
        self.compilation_cache = {}

    def cache(self, k: str, b: BindableContract):
        self.compilation_cache[k] = b

    def uncache(self, k: str) -> Optional[BindableContract]:
        try:
            return self.compilation_cache[k]
        except KeyError:
            return None
