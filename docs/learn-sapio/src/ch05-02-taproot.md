# No Taproot

Currently Taproot is not active on Bitcoin and there is no deployment planned
for it.

Sapio scripts can become very large in size, and would greatly benefit from
being able to split up and merkelize the logic into smaller satisfiable
chunks. This makes it economical to use Sapio.

The compiler is currently relatively naive about this, and unknown (or worse,
unchecked) errors might occur as a result of pushing these limits. Hopefully,
`rust-miniscript` should catch such errors, but a malicious actor might be
able to trigger an unknown unsatisfiable script.

As such, Sapio is probably ill-advisable to use at writing, but this will
hopefully change in the future.

## Emulation?

In theory, Taproot could also be emulated in a similar manner to CTV. You
would run a server that would send a replacement key to use instead of the
Taproot key, and then the emulator would sign off on the transaction if the user
could provide a satisfaction.

Fortunately, it does seem that Taproot will be active within the next year,
so such measures are not yet required.

## Taproot Optimizations

With Taproot comes the opportunity to [Huffman
Code](https://en.wikipedia.org/wiki/Huffman_coding) spending paths to
decrease fees even further. Sapio currently uses `rust-miniscript` Policy
language to generate spending conditions, so Sapio should be able to carry
metadata from the programmer about the likelihood of various paths being
taken, but this currently only is used within a script as opposed to the
Tapscript tree itself.