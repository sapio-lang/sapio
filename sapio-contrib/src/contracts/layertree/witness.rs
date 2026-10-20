//! Canonical initial stacks for the dense MID reference scripts.
//!
//! Helpers return the next tree and the exact framed interpreter stack.
//! Signatures may be empty while constructing a proposal; the interpreter
//! remains the authority that verifies signatures and transition rules.

use super::tree::{Leaf, Node, Proof, Tree, MAX_SUM, SEQ_FINAL};

/// Initial main-stack elements, in bottom-to-top order.
pub type Stack = Vec<Vec<u8>>;
/// The resulting tree and the initial stack authorizing its transition.
pub type Prepared = (Tree, Stack);

/// Encode a minimal nonnegative four-byte CScriptNum.
pub fn script_number(value: u64) -> Result<Vec<u8>, &'static str> {
    if value > MAX_SUM {
        return Err("number exceeds MID range");
    }
    let mut number = value;
    let mut bytes = Vec::new();
    while number != 0 {
        bytes.push(number as u8);
        number >>= 8;
    }
    if bytes.last().is_some_and(|b| b & 0x80 != 0) {
        bytes.push(0);
    }
    Ok(bytes)
}

/// Construct the checked zero padding used by the reference PADZ gadget.
pub fn pad(value: u64) -> Result<[Vec<u8>; 2], &'static str> {
    let width = script_number(value)?.len();
    Ok([vec![0; 4 - width], vec![0; width]])
}

/// Supply seven leaf fields and a checked numeric representation of its amount.
pub fn leaf(leaf: &Leaf) -> Result<Stack, &'static str> {
    leaf.validate()?;
    let bytes = leaf.serialize();
    let mut out = vec![
        bytes[..8].to_vec(),
        bytes[8..40].to_vec(),
        bytes[40..72].to_vec(),
        bytes[72..104].to_vec(),
        bytes[104..108].to_vec(),
        bytes[108..112].to_vec(),
        bytes[112..].to_vec(),
        script_number(leaf.amount)?,
    ];
    out.extend(pad(leaf.amount)?);
    Ok(out)
}

/// Append the lease's numeric representation for an owner exit or eviction.
pub fn leaf_exit(record: &Leaf) -> Result<Stack, &'static str> {
    let mut out = leaf(record)?;
    out.push(script_number(record.evict_after as u64)?);
    out.extend(pad(record.evict_after as u64)?);
    Ok(out)
}

/// Level D-1 is deepest in the initial stack; level zero executes first.
pub fn levels(path: &Proof, mut old: Node, mut new: Node) -> Result<Stack, &'static str> {
    let mut per_level = Vec::with_capacity(path.0.len());
    for sibling in &path.0 {
        let mut items = Vec::with_capacity(9);
        items.extend(pad(old.sum)?);
        items.extend(pad(new.sum)?);
        items.push(if sibling.right { vec![1] } else { vec![] });
        items.push(sibling.node.hash.to_vec());
        items.push(script_number(sibling.node.sum)?);
        items.extend(pad(sibling.node.sum)?);
        per_level.push(items);
        old = if sibling.right {
            Node::parent(sibling.node, old)?
        } else {
            Node::parent(old, sibling.node)?
        };
        new = if sibling.right {
            Node::parent(sibling.node, new)?
        } else {
            Node::parent(new, sibling.node)?
        };
    }
    Ok(per_level.into_iter().rev().flatten().collect())
}

fn finish(mut stack: Stack, tree: &Tree, merkle_root: [u8; 32]) -> Stack {
    stack.push(tree.root().hash.to_vec());
    stack.push(merkle_root.to_vec());
    stack
}

/// Sequential exits prove each leaf against the preceding removal's root.
pub fn exit(
    tree: &Tree,
    exits: &[(u32, bool, Vec<u8>)],
    merkle_root: [u8; 32],
) -> Result<Prepared, &'static str> {
    if exits.is_empty() || exits.len() > 2 {
        return Err("supported exit count is one or two");
    }
    let mut next = tree.clone();
    let mut blocks = Vec::with_capacity(exits.len());
    for (step, (position, is_evict, signature)) in exits.iter().enumerate() {
        let record = next
            .leaf(*position)
            .cloned()
            .ok_or("exit position is empty")?;
        let path = next.proof(*position)?;
        let mut block = if step == 0 {
            pad(tree.root().sum)?.to_vec()
        } else {
            vec![]
        };
        block.extend(levels(&path, record.node(), Node::default())?);
        block.extend(leaf_exit(&record)?);
        block.push(signature.clone());
        block.push(if *is_evict { vec![1] } else { vec![] });
        blocks.push(block);
        next.set(*position, None)?;
    }
    let mut stack = pad(next.root().sum)?.to_vec();
    stack.extend(blocks.into_iter().rev().flatten());
    Ok((next, finish(stack, tree, merkle_root)))
}

/// Graft verified leaves into an aligned empty subtree. Positions are absolute.
pub fn join(
    tree: &Tree,
    height: u8,
    index: u32,
    records: &[(u32, Leaf)],
    operator_signature: &[u8],
    merkle_root: [u8; 32],
) -> Result<Prepared, &'static str> {
    if height != 0 && height != 2 {
        return Err("supported join height is zero or two");
    }
    let old = tree.subtree(height, index)?;
    if old != Node::empty(height) {
        return Err("join subtree is occupied");
    }
    if records.is_empty() {
        return Err("join must fund a nonempty subtree");
    }
    let base = index << height;
    let mut next = tree.clone();
    let mut seen = std::collections::BTreeSet::new();
    for (position, record) in records {
        if *position < base || *position >= base + (1 << height) || !seen.insert(*position) {
            return Err("join positions must be distinct and inside the subtree");
        }
        next.set(*position, Some(record.clone()))?;
    }
    let new = next.subtree(height, index)?;
    let mut stack = pad(next.root().sum)?.to_vec();
    stack.extend(pad(tree.root().sum)?);
    stack.extend(levels(&tree.proof_subtree(height, index)?, old, new)?);
    stack.push(new.hash.to_vec());
    stack.push(script_number(new.sum)?);
    stack.push(operator_signature.to_vec());
    Ok((next, finish(stack, tree, merkle_root)))
}

/// Replace one record under the old owner and operator signatures.
pub fn update(
    tree: &Tree,
    position: u32,
    new: Leaf,
    owner_signature: &[u8],
    operator_signature: &[u8],
    merkle_root: [u8; 32],
) -> Result<Prepared, &'static str> {
    let old = tree.leaf(position).ok_or("update position is empty")?;
    let mut next = tree.clone();
    next.set(position, Some(new.clone()))?;
    let mut stack = pad(next.root().sum)?.to_vec();
    stack.extend(pad(tree.root().sum)?);
    stack.extend(levels(&tree.proof(position)?, old.node(), new.node())?);
    stack.extend(leaf(old)?);
    stack.extend(leaf(&new)?);
    stack.push(owner_signature.to_vec());
    stack.push(operator_signature.to_vec());
    Ok((next, finish(stack, tree, merkle_root)))
}

/// A CLAIM, hashlocked CLAIM, or SPLIT witness request.
pub struct Carve<'a> {
    /// Position whose balance funds the new leaf.
    pub source_position: u32,
    /// Position proven empty after applying the source reduction.
    pub claim_position: u32,
    /// Complete new member record covered by the authorizing signature.
    pub claim: Leaf,
    /// Owner transaction signature for SPLIT or a raw voucher signature.
    pub signature: &'a [u8],
    /// Present only for the separately tagged hashlocked voucher script.
    pub preimage: Option<[u8; 32]>,
}

/// Prove both ordered replacements and preserve eviction and dust invariants.
pub fn carve(
    tree: &Tree,
    carve: Carve<'_>,
    dust_min: u64,
    merkle_root: [u8; 32],
) -> Result<Prepared, &'static str> {
    let source = tree
        .leaf(carve.source_position)
        .ok_or("source position is empty")?;
    carve.claim.validate()?;
    let residual = source
        .amount
        .checked_sub(carve.claim.amount)
        .ok_or("claim exceeds source")?;
    if source.seq >= SEQ_FINAL {
        return Err("source leaf is non-claimable");
    }
    if dust_min == 0 || carve.claim.amount < dust_min || residual != 0 && residual < dust_min {
        return Err("carve amount or residual is below the dust floor");
    }
    if carve.claim.evictor_key != source.evictor_key || carve.claim.evict_after > source.evict_after
    {
        return Err("carve cannot weaken eviction terms");
    }
    let mut intermediate = tree.clone();
    let new_source = if residual == 0 {
        None
    } else {
        let mut record = source.clone();
        record.amount = residual;
        record.seq += 1;
        Some(record)
    };
    let new_source_node = new_source.as_ref().map(Leaf::node).unwrap_or_default();
    intermediate.set(carve.source_position, new_source)?;
    if intermediate.leaf(carve.claim_position).is_some() {
        return Err("claim position is occupied");
    }
    let mut next = intermediate.clone();
    next.set(carve.claim_position, Some(carve.claim.clone()))?;
    let mut stack = pad(next.root().sum)?.to_vec();
    stack.extend(levels(
        &intermediate.proof(carve.claim_position)?,
        Node::default(),
        carve.claim.node(),
    )?);
    stack.extend(pad(tree.root().sum)?);
    stack.extend(levels(
        &tree.proof(carve.source_position)?,
        source.node(),
        new_source_node,
    )?);
    stack.extend(leaf(source)?);
    stack.extend(leaf(&carve.claim)?);
    stack.push(script_number(source.seq as u64)?);
    stack.extend(pad(source.seq as u64)?);
    stack.extend(pad(source.seq as u64 + 1)?);
    stack.extend(pad(residual)?);
    stack.push(script_number(source.evict_after as u64)?);
    stack.extend(pad(source.evict_after as u64)?);
    stack.push(script_number(carve.claim.evict_after as u64)?);
    stack.extend(pad(carve.claim.evict_after as u64)?);
    stack.push(carve.signature.to_vec());
    if let Some(preimage) = carve.preimage {
        stack.push(preimage.to_vec());
    }
    Ok((next, finish(stack, tree, merkle_root)))
}

/// Sweep a height-one or height-two cohort with exact ordered payouts.
pub fn evict_subtree(
    tree: &Tree,
    height: u8,
    index: u32,
    operator_signature: &[u8],
    merkle_root: [u8; 32],
) -> Result<Prepared, &'static str> {
    if !(1..=2).contains(&height) {
        return Err("supported sweep height is one or two");
    }
    let old = tree.subtree(height, index)?;
    let base = index << height;
    let mut next = tree.clone();
    for position in base..base + (1 << height) {
        next.set(position, None)?;
    }
    let mut stack = pad(next.root().sum)?.to_vec();
    stack.extend(pad(tree.root().sum)?);
    stack.extend(levels(
        &tree.proof_subtree(height, index)?,
        old,
        Node::empty(height),
    )?);
    // The mirrored postorder hash pass consumes the right pair's pad first.
    if height == 2 {
        stack.extend(pad(tree.subtree(1, index * 2)?.sum)?);
        stack.extend(pad(tree.subtree(1, index * 2 + 1)?.sum)?);
    }
    for position in (base..base + (1 << height)).rev() {
        if let Some(record) = tree.leaf(position) {
            stack.extend(leaf_exit(record)?);
            stack.push(vec![1]);
        } else {
            stack.push(vec![]);
        }
    }
    stack.push(operator_signature.to_vec());
    Ok((next, finish(stack, tree, merkle_root)))
}

/// A four-position cohort rewrite, with signatures indexed within the cohort.
pub struct Rewrite<'a> {
    /// Index of the aligned height-two subtree.
    pub index: u32,
    /// Complete post-transition contents, in ascending position order.
    pub new: [Option<Leaf>; 4],
    /// Old owner signatures, included only when the Script requires them.
    pub signatures: [&'a [u8]; 4],
}

/// Rewrite a cohort using canonical unchanged and amount-only encodings.
pub fn rewrite(
    tree: &Tree,
    rewrite: Rewrite<'_>,
    operator_signature: &[u8],
    merkle_root: [u8; 32],
) -> Result<Prepared, &'static str> {
    let old = tree.subtree(2, rewrite.index)?;
    let base = rewrite.index << 2;
    let mut next = tree.clone();
    // Remove the old cohort before inserting replacements: a cooperative
    // transfer may move an identity between positions within this cohort.
    for position in base..base + 4 {
        next.set(position, None)?;
    }
    for (offset, record) in rewrite.new.iter().enumerate() {
        if let Some(record) = record {
            next.set(base + offset as u32, Some(record.clone()))?;
        }
    }
    let new = next.subtree(2, rewrite.index)?;
    let mut stack = pad(next.root().sum)?.to_vec();
    stack.extend(pad(tree.root().sum)?);
    stack.extend(levels(&tree.proof_subtree(2, rewrite.index)?, old, new)?);
    for index in [rewrite.index * 2, rewrite.index * 2 + 1] {
        stack.extend(pad(next.subtree(1, index)?.sum)?);
        stack.extend(pad(tree.subtree(1, index)?.sum)?);
    }
    for offset in (0..4).rev() {
        let old = tree.leaf(base + offset as u32);
        let new = rewrite.new[offset].as_ref();
        let same_body = old
            .zip(new)
            .is_some_and(|(old, new)| old.serialize()[8..] == new.serialize()[8..]);
        let raised = old.zip(new).is_some_and(|(old, new)| {
            same_body && new.amount >= old.amount && old.seq != SEQ_FINAL
        });
        if old.is_some() && old != new && !raised {
            stack.push(rewrite.signatures[offset].to_vec());
        }
        if old.is_some() && old == new {
            stack.push(vec![2]);
        } else if same_body {
            let amount = new.expect("equal leaf bodies require a new leaf").amount;
            stack.push(script_number(amount)?);
            stack.extend(pad(amount)?);
            stack.push(vec![3]);
        } else {
            stack.extend(rewrite_slot(new)?);
        }
        stack.extend(rewrite_slot(old)?);
    }
    stack.push(operator_signature.to_vec());
    Ok((next, finish(stack, tree, merkle_root)))
}

fn rewrite_slot(record: Option<&Leaf>) -> Result<Stack, &'static str> {
    if let Some(record) = record {
        let mut out = leaf(record)?;
        out.push(vec![1]);
        Ok(out)
    } else {
        Ok(vec![vec![]])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::hex::DisplayHex;

    #[test]
    fn rewrite_stack_matches_the_pinned_reference_builder() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("upstream_vectors.json")).unwrap();
        let fixture = &fixture["rewrite_witness"];
        let mut tree = Tree::new(4).unwrap();
        for (position, record) in fixture["old"].as_object().unwrap() {
            tree.set(
                position.parse().unwrap(),
                Some(serde_json::from_value(record.clone()).unwrap()),
            )
            .unwrap();
        }
        let new = std::array::from_fn(|offset| {
            serde_json::from_value(fixture["new"][offset].clone()).unwrap()
        });
        let (next, stack) = rewrite(
            &tree,
            Rewrite {
                index: 0,
                new,
                signatures: [b"owner-zero", b"", b"owner-two", b""],
            },
            b"operator",
            [0x11; 32],
        )
        .unwrap();
        let expected: Vec<String> = serde_json::from_value(fixture["stack"].clone()).unwrap();
        assert_eq!(
            stack
                .iter()
                .map(|item| item.to_lower_hex_string())
                .collect::<Vec<_>>(),
            expected
        );
        assert_eq!(
            next.root().hash.to_lower_hex_string(),
            fixture["next_root"]["hash"].as_str().unwrap()
        );
        assert_eq!(
            next.root().sum,
            fixture["next_root"]["sum"].as_u64().unwrap()
        );
    }
}
