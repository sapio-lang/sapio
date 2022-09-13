// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! The host's handle to communicate with a WASM Module

use super::wasm_cache;
use crate::CreateArgs;
use sapio_ctv_emulator_trait::NullEmulator;
use std::cell::Cell;
use std::collections::BTreeMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
pub use wasm::*;
use wasmer::{imports, Function, ImportObject, Instance, LazyInit, MemoryView, Module, Store};
pub use wasmer_cache::Hash as WASMCacheID;

mod wasm;
