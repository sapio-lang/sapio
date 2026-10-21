//! The published MID tier, assembled with typed runtime parameters.
//!
//! The generated fragments retain the reference's canonical field checks,
//! arithmetic and intermediate-root checks. They are byte-identical to the
//! pinned upstream generator; no transition logic lives in the WASM host.

use super::tree::MAX_SUM;
use bitcoin::ScriptBuf;
use serde::{Deserialize, Serialize};

#[path = "script_fragments.rs"]
mod fragments;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
/// Fixed parameters shared by every state in this deployment.
pub struct Config {
    /// Dense tree depth, from one through sixteen.
    pub depth: u8,
    /// Admission, update and cohort eviction authority.
    pub operator: bitcoin::XOnlyPublicKey,
    /// Unique voucher lineage identifier, derived before tree creation.
    pub tree_id: [u8; 32],
    /// Minimum amount of a carved leaf and any nonzero residual.
    pub dust_min: u64,
}

impl Config {
    /// Reject unsupported depths and arithmetic bounds.
    pub fn validate(&self) -> Result<(), &'static str> {
        if !(1..=16).contains(&self.depth) {
            return Err("supported MID depth is 1 through 16");
        }
        if self.dust_min == 0 || self.dust_min > MAX_SUM {
            return Err("dust floor is outside the MID range");
        }
        Ok(())
    }

    /// The complete option set remains unchanged across states.
    pub fn transitions(&self) -> Vec<Transition> {
        let mut transitions = vec![
            Transition::Exit { count: 1 },
            Transition::Exit { count: 2 },
            Transition::Join { height: 0 },
            Transition::Update,
            Transition::Claim,
            Transition::ClaimHashlock,
            Transition::Split,
            Transition::EvictSubtree { height: 1 },
        ];
        if self.depth >= 2 {
            transitions.push(Transition::Join { height: 2 });
            transitions.push(Transition::EvictSubtree { height: 2 });
            transitions.push(Transition::Rewrite { height: 2 });
        }
        transitions
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
/// A supported member of the fixed MID transition option set.
pub enum Transition {
    /// One or two sequential owner exits or leased evictions.
    Exit {
        /// Number of independently authorized exits.
        count: u8,
    },
    /// Graft a verified subtree into an aligned empty position.
    Join {
        /// Subtree height, zero or two.
        height: u8,
    },
    /// Replace one leaf with the owner and operator's consent.
    Update,
    /// Redeem a bearer voucher into a new leaf.
    Claim,
    /// Redeem a separately tagged voucher with a 32-byte preimage.
    ClaimHashlock,
    /// Carve a new leaf with the source owner's transaction signature.
    Split,
    /// Sweep every expired member of an aligned cohort, paying each exactly.
    EvictSubtree {
        /// Subtree height, one or two.
        height: u8,
    },
    /// Rewrite a cohort with the operator and all changed owners' consent.
    Rewrite {
        /// Supported cohort height, two.
        height: u8,
    },
}

/// Assemble the exact pinned reference Script for this transition.
pub fn script(config: &Config, transition: Transition) -> Result<ScriptBuf, &'static str> {
    config.validate()?;
    let bytes = match transition {
        Transition::Exit { count: 1 } => fragments::exit_1(config),
        Transition::Exit { count: 2 } => fragments::exit_2(config),
        Transition::Join { height: 0 } => fragments::join_0(config),
        Transition::Join { height: 2 } if config.depth >= 2 => fragments::join_2(config),
        Transition::Update => fragments::update(config),
        Transition::Claim => fragments::claim(config),
        Transition::ClaimHashlock => fragments::claim_hashlock(config),
        Transition::Split => fragments::split(config),
        Transition::EvictSubtree { height: 1 } => fragments::sweep_1(config),
        Transition::EvictSubtree { height: 2 } if config.depth >= 2 => fragments::sweep_2(config),
        Transition::Rewrite { height: 2 } if config.depth >= 2 => fragments::rewrite_2(config),
        _ => return Err("unsupported transition parameters"),
    };
    if bytes.len() > 10_000 {
        return Err("transition exceeds the bounded interpreter Script limit");
    }
    Ok(ScriptBuf::from_bytes(bytes))
}

fn push_number(out: &mut Vec<u8>, value: i64) {
    if value == 0 {
        out.push(0);
    } else if (1..=16).contains(&value) {
        out.push(0x50 + value as u8);
    } else {
        let mut number = value as u64;
        let mut bytes = Vec::new();
        while number != 0 {
            bytes.push(number as u8);
            number >>= 8;
        }
        if bytes.last().is_some_and(|b| b & 0x80 != 0) {
            bytes.push(0);
        }
        push_data(out, &bytes);
    }
}

fn push_data(out: &mut Vec<u8>, bytes: &[u8]) {
    match bytes {
        [] => out.push(0),
        [value @ 1..=16] => out.push(0x50 + *value),
        _ if bytes.len() <= 75 => out.push(bytes.len() as u8),
        _ if bytes.len() <= 255 => out.extend_from_slice(&[0x4c, bytes.len() as u8]),
        _ => {
            out.push(0x4d);
            out.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
        }
    }
    if !(bytes.is_empty() || bytes.len() == 1 && (1..=16).contains(&bytes[0])) {
        out.extend_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hex::{DisplayHex, FromHex};
    use std::str::FromStr;

    #[test]
    fn published_mid_scripts_match_upstream_golden_bytes() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("upstream_vectors.json")).unwrap();
        let config = Config {
            depth: 4,
            operator: bitcoin::XOnlyPublicKey::from_str(vectors["operator_key"].as_str().unwrap())
                .unwrap(),
            tree_id: <[u8; 32]>::from_hex(vectors["tree_id"].as_str().unwrap()).unwrap(),
            dust_min: 330,
        };
        for (name, transition) in [
            ("EXIT_1", Transition::Exit { count: 1 }),
            ("JOIN_0", Transition::Join { height: 0 }),
            ("JOIN_2", Transition::Join { height: 2 }),
            ("UPDATE", Transition::Update),
            ("CLAIM", Transition::Claim),
            ("CLAIM_H", Transition::ClaimHashlock),
            ("SPLIT", Transition::Split),
            ("EVICT_SUBTREE_1", Transition::EvictSubtree { height: 1 }),
            ("EVICT_SUBTREE_2", Transition::EvictSubtree { height: 2 }),
            ("REWRITE_2", Transition::Rewrite { height: 2 }),
        ] {
            let generated = script(&config, transition).unwrap();
            assert_eq!(
                generated.as_bytes().to_lower_hex_string(),
                vectors["scripts"][name]["hex"].as_str().unwrap(),
                "{name}"
            );
        }
        // This is a resource guarantee of the supported profile, independent
        // of the witness constructors and their local validation.
        for depth in 1..=16 {
            let config = Config {
                depth,
                ..config.clone()
            };
            for transition in config.transitions() {
                script(&config, transition).unwrap();
            }
        }
    }
}
