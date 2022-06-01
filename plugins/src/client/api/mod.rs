// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

///! Wraps the external API with friendly methods
use super::*;
use crate::plugin_handle::PluginHandle;
use core::convert::TryFrom;
use sapio::contract::CompilationError;
use sapio_base::effects::EffectPath;
use sapio_trait::SapioJSONTrait;
use std::marker::PhantomData;

pub mod emulator;
pub mod handle;
pub mod lookup;
pub mod util;

pub use emulator::*;
pub use handle::*;
pub use lookup::*;
pub use util::*;
