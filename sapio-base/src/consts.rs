// TODO: Fix the const fn version
use bitcoin::blockdata::opcodes::all;
///pub const FALSE_PATTERN: &[u8] = &[all::OP_PUSHBYTES_0.into_u8()];
pub const FALSE_PATTERN: &[u8] = &[0];
///pub const TRUE_PATTERN: &[u8] = &[all::OP_PUSHNUM_1.into_u8()];
pub const TRUE_PATTERN: &[u8] = &[81];
