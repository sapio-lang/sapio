# TUX

[Tux](https://github.com/sapio-lang/tux) is an in-development graphical user
interface for Sapio.

Currently, Tux works by communicating to a rust websocket server that manages
compiler sessions. This is being rearchitected to work based on managing a
WASM plugin directory, so that users can more readily add contracts of their
choosing.

Contracts packaged for TUX may have some additional constraints or
functionality for aiding in the generation of a UX.