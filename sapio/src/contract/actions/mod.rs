// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The different types of functionality a contract can define.
use super::CompilationError;
use super::Context;
use super::TxTmplIt;
pub mod guard;
pub use guard::*;
pub mod then;
pub use then::*;
pub mod conditional_compile;
pub use conditional_compile::*;
pub mod finish;
pub use finish::*;
