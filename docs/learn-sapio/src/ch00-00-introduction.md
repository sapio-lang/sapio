# Introduction

Sapio is a Rust framework for describing Bitcoin contracts as transaction graphs.
It separates constructing transactions, defining their spending policies and
supplying the signatures or program evidence needed to complete a selected spend.

The first lesson uses the maintained generated starter and its tested contract
source. Later chapters include historical examples and conceptual sketches;
use the current repository guides when an older sketch differs from the public
API. Rendering this book does not compile every code block.

## Who is Sapio For?

Sapio is for anyone who wants to build with Bitcoin. That spans students
demonstrating research concepts, corporations working on custody solutions,
and developers improving open source solutions. Sapio is not a Solidity
equivalent. The programming model is _very_ different. But it does help
anyone trying to solve a transactional protocol for Bitcoin solve it
elegantly.

The developer preview uses synthetic funding and public demonstration keys.
Native covenant research and oracle-based emulation have distinct enforcement
assumptions. Completing the lesson does not establish production readiness;
the [release-readiness record](https://github.com/sapio-lang/sapio/blob/master/docs/RELAUNCH.md)
lists the remaining gates.

## What will I learn if I read this book?

This book is intended to teach you how to think about programming Sapio
contracts. The book contains some exercises (that are heavily encouraged)
that should instigate your understanding of how to build smart contracts
for Bitcoin.

If you go through the chapters in order and complete all the exercises you
should develop a firm grasp of how to use Sapio, how it works, and how it
will progress over time. You will also have sufficient understanding to
contribute back meaningfully to the open source project.
