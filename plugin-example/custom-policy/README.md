# Custom policy compiler

`ArithmeticSigner` implements `sapio_base::policy::PolicyCompiler` and emits
`<owner> CHECKSIG 1ADD 2 NUMEQUAL`, a signature predicate outside Miniscript's
language. Sapio combines it with a native co-signer and the payment covenant.
The payment reserves 1,000 satoshis as fees and pays the co-signer the remainder.

The raw backend owns witness construction and signature semantics. Checked
fragments preserve composition boundaries; they do not imply an automatic
finalizer or a satisfaction-weight estimate. The native custom-policy vectors
and Bitcoin Core harness exercise explicit witnesses for this arithmetic form.
