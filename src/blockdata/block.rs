// Rust Bitcoin Library
// Written in 2014 by
//     Andrew Poelstra <apoelstra@wpsoftware.net>
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! Bitcoin blocks.
//!
//! A block is a bundle of transactions with a proof-of-work attached,
//! which commits to an earlier block to form the blockchain. This
//! module describes structures and functions needed to describe
//! these blocks and the blockchain.
//!

use prelude::*;

use core::fmt;

use util;
use util::Error::{BlockBadTarget, BlockBadProofOfWork};
use util::hash::bitcoin_merkle_root;
use hashes::{Hash, HashEngine};
use hash_types::{Wtxid, BlockHash, TxMerkleNode, WitnessMerkleNode, WitnessCommitment};
use util::uint::Uint256;
use consensus::{encode, Decodable, Encodable};
use network::constants::Network;
use blockdata::transaction::Transaction;
use blockdata::constants::{max_target, WITNESS_SCALE_FACTOR};
use blockdata::script;
use VarInt;
use io::{self};

/// A block header, which contains all the block's information except
/// the actual transactions
#[derive(PartialEq, Eq, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct BlockHeader {
    /// The protocol version. Should always be 1.
    pub version: Version,
    /// Reference to the previous block in the chain.
    pub prev_blockhash: BlockHash,
    /// The root hash of the merkle tree of transactions in the block.
    pub merkle_root: TxMerkleNode,
    /// The timestamp of the block, as claimed by the miner.
    pub time: u32,
    /// The target value below which the blockhash must lie, encoded as a
    /// a float (with well-defined rounding, of course).
    pub bits: u32,
    /// The nonce, selected to obtain a low enough blockhash.
    pub nonce: u32,    
    /// AuxPow header data
    pub aux_data: Option<AuxPow>,
}

//impl_consensus_encoding!(Header, version, prev_blockhash, merkle_root, time, bits, nonce);
impl Decodable for BlockHeader {
    fn consensus_decode_from_finite_reader<R: io::Read + ?Sized>(
        reader: &mut R,
    ) -> Result<Self, encode::Error> {
        let base = SimpleHeader::consensus_decode_from_finite_reader(reader)?;
        if (base.version.0 & 0x100) == 0 {
            return Ok(BlockHeader {
                version: base.version,
                prev_blockhash: base.prev_blockhash,
                merkle_root: base.merkle_root,
                time: base.time,
                bits: base.bits,
                nonce: base.nonce,
                aux_data: None,
            });
        } else {
            let aux_data = AuxPow::consensus_decode_from_finite_reader(reader)?;
            Ok(BlockHeader {
                version: base.version,
                prev_blockhash: base.prev_blockhash,
                merkle_root: base.merkle_root,
                time: base.time,
                bits: base.bits,
                nonce: base.nonce,
                aux_data: Some(aux_data),
            })
        }
    }

    fn consensus_decode<D: io::Read>(reader: D) -> Result<Self, encode::Error> {
        use crate::io::Read as _;
        let mut r = reader.take(encode::MAX_VEC_SIZE as u64);
        let thing = SimpleHeader::consensus_decode(r.by_ref())?;
        if (thing.version.0 & 0x100) == 0 {
            return Ok(BlockHeader {
                version: thing.version,
                prev_blockhash: thing.prev_blockhash,
                merkle_root: thing.merkle_root,
                time: thing.time,
                bits: thing.bits,
                nonce: thing.nonce,
                aux_data: None,
            });
        } else {
            let aux_data = AuxPow::consensus_decode(r.by_ref())?;
            Ok(BlockHeader {
                version: thing.version,
                prev_blockhash: thing.prev_blockhash,
                merkle_root: thing.merkle_root,
                time: thing.time,
                bits: thing.bits,
                nonce: thing.nonce,
                aux_data: Some(aux_data),
            })
        }
    }
}

impl Encodable for BlockHeader {
    fn consensus_encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        let mut len = self.version.consensus_encode(writer)?;
        len += self.prev_blockhash.consensus_encode(writer)?;
        len += self.merkle_root.consensus_encode(writer)?;
        len += self.time.consensus_encode(writer)?;
        len += self.bits.consensus_encode(writer)?;
        len += self.nonce.consensus_encode(writer)?;
        if (self.version.0 & 0x100) != 0 {
            len += self.aux_data.as_ref().unwrap().consensus_encode(writer)?;
        }
        Ok(len)
    }
}


#[derive(Copy, PartialEq, Eq, Clone, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SimpleHeader {
    /// Block version, now repurposed for soft fork signalling.
    pub version: Version,
    /// Reference to the previous block in the chain.
    pub prev_blockhash: BlockHash,
    /// The root hash of the merkle tree of transactions in the block.
    pub merkle_root: TxMerkleNode,
    /// The timestamp of the block, as claimed by the miner.
    pub time: u32,
    /// The target value below which the blockhash must lie.
    pub bits: u32,
    /// The nonce, selected to obtain a low enough blockhash.
    pub nonce: u32,
}

#[derive(Copy, PartialEq, Eq, Clone, Debug, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Version(pub i32);

impl Version {
    /// The original Bitcoin Block v1.
    pub const ONE: Self = Self(1);
    /// BIP-34 Block v2.
    pub const TWO: Self = Self(2);
    /// BIP-9 compatible version number that does not signal for any softforks.
    pub const NO_SOFT_FORK_SIGNALLING: Self = Self(Self::USE_VERSION_BITS as i32);
    /// BIP-9 soft fork signal bits mask.
    const VERSION_BITS_MASK: u32 = 0x1FFF_FFFF;
    /// 32bit value starting with `001` to use version bits.
    ///
    /// The value has the top three bits `001` which enables the use of version bits to signal for soft forks.
    const USE_VERSION_BITS: u32 = 0x2000_0000;
    /// Creates a [`Version`] from a signed 32 bit integer value.
    ///
    /// This is the data type used in consensus code in Bitcoin Core.
    pub fn from_consensus(v: i32) -> Self {
        Version(v)
    }

    /// Returns the inner `i32` value.
    ///
    /// This is the data type used in consensus code in Bitcoin Core.
    pub fn to_consensus(self) -> i32 { self.0 }

    /// Checks whether the version number is signalling a soft fork at the given bit.
    ///
    /// A block is signalling for a soft fork under BIP-9 if the first 3 bits are `001` and
    /// the version bit for the specific soft fork is toggled on.
    pub fn is_signalling_soft_fork(&self, bit: u8) -> bool {
        // Only bits [0, 28] inclusive are used for signalling.
        if bit > 28 {
            return false;
        }
        // To signal using version bits, the first three bits must be `001`.
        if (self.0 as u32) & !Self::VERSION_BITS_MASK != Self::USE_VERSION_BITS {
            return false;
        }
        // The bit is set if signalling a soft fork.
        (self.0 as u32 & Self::VERSION_BITS_MASK) & (1 << bit) > 0
    }
}

impl Encodable for Version {
    fn consensus_encode<W: io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, io::Error> {
        self.0.consensus_encode(w)
    }
}
impl Decodable for Version {
    fn consensus_decode<R: io::Read + ?Sized>(r: &mut R) -> Result<Self, encode::Error> {
        Decodable::consensus_decode(r).map(Version)
    }
}

impl_consensus_encoding!(SimpleHeader, version, prev_blockhash, merkle_root, time, bits, nonce);

impl BlockHeader {
    pub fn to_simple_header(&self) -> SimpleHeader {
        SimpleHeader {
            version: self.version,
            prev_blockhash: self.prev_blockhash,
            merkle_root: self.merkle_root,
            time: self.time,
            bits: self.bits,
            nonce: self.nonce,
        }
    }

    /// Returns the block hash.
    pub fn block_hash(&self) -> BlockHash {
        let mut engine = BlockHash::engine();
        self.to_simple_header().consensus_encode(&mut engine).expect("engines don't error");
        BlockHash::from_engine(engine)
    }

    /// Computes the target [0, T] that a blockhash must land in to be valid.
    pub fn target(&self) -> Uint256 {
        Self::u256_from_compact_target(self.bits)
    }

    /// Computes the target value in [`Uint256`] format, from a compact representation.
    ///
    /// [`Uint256`]: ../../util/uint/struct.Uint256.html
    ///
    /// ```
    /// use bitcoin::blockdata::block::BlockHeader;
    ///
    /// assert_eq!(0x1d00ffff,
    ///     BlockHeader::compact_target_from_u256(
    ///         &BlockHeader::u256_from_compact_target(0x1d00ffff)
    ///     )
    /// );
    /// ```
    pub fn u256_from_compact_target(bits: u32) -> Uint256 {
        // This is a floating-point "compact" encoding originally used by
        // OpenSSL, which satoshi put into consensus code, so we're stuck
        // with it. The exponent needs to have 3 subtracted from it, hence
        // this goofy decoding code:
        let (mant, expt) = {
            let unshifted_expt = bits >> 24;
            if unshifted_expt <= 3 {
                ((bits & 0xFFFFFF) >> (8 * (3 - unshifted_expt as usize)), 0)
            } else {
                (bits & 0xFFFFFF, 8 * ((bits >> 24) - 3))
            }
        };

        // The mantissa is signed but may not be negative
        if mant > 0x7FFFFF {
            Default::default()
        } else {
            Uint256::from_u64(mant as u64).unwrap() << (expt as usize)
        }
    }

    /// Computes the target value in float format from Uint256 format.
    pub fn compact_target_from_u256(value: &Uint256) -> u32 {
        let mut size = (value.bits() + 7) / 8;
        let mut compact = if size <= 3 {
            (value.low_u64() << (8 * (3 - size))) as u32
        } else {
            let bn = *value >> (8 * (size - 3));
            bn.low_u32()
        };

        if (compact & 0x00800000) != 0 {
            compact >>= 8;
            size += 1;
        }

        compact | (size << 24) as u32
    }

    /// Computes the popular "difficulty" measure for mining.
    pub fn difficulty(&self, network: Network) -> u64 {
        (max_target(network) / self.target()).low_u64()
    }

    /// Checks that the proof-of-work for the block is valid, returning the block hash.
    pub fn validate_pow(&self, required_target: &Uint256) -> Result<BlockHash, util::Error> {
        let target = &self.target();
        if target != required_target {
            return Err(BlockBadTarget);
        }
        let block_hash = self.block_hash();
        let mut ret = [0u64; 4];
        util::endian::bytes_to_u64_slice_le(block_hash.as_inner(), &mut ret);
        let hash = &Uint256(ret);
        if hash <= target { Ok(block_hash) } else { Err(BlockBadProofOfWork) }
    }

    /// Returns the total work of the block.
    pub fn work(&self) -> Uint256 {
        // 2**256 / (target + 1) == ~target / (target+1) + 1    (eqn shamelessly stolen from bitcoind)
        let mut ret = !self.target();
        let mut ret1 = self.target();
        ret1.increment();
        ret = ret / ret1;
        ret.increment();
        ret
    }

    pub fn get_size(&self) -> usize {
        if self.aux_data.is_none() {
            return 80
        }else{
            80 + self.aux_data.as_ref().unwrap().get_size()
        }
    }
}

/// A Bitcoin block, which is a collection of transactions with an attached
/// proof of work.
#[derive(PartialEq, Eq, Clone, Debug)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Block {
    /// The block header
    pub header: BlockHeader,
    /// List of transactions contained in the block
    pub txdata: Vec<Transaction>
}

impl_consensus_encoding!(Block, header, txdata);

impl Block {
    /// Returns the block hash.
    pub fn block_hash(&self) -> BlockHash {
        self.header.block_hash()
    }

    /// check if merkle root of header matches merkle root of the transaction list
    pub fn check_merkle_root(&self) -> bool {
        match self.compute_merkle_root() {
            Some(merkle_root) => self.header.merkle_root == merkle_root,
            None => false,
        }
    }

    /// Checks if witness commitment in coinbase matches the transaction list.
    pub fn check_witness_commitment(&self) -> bool {
        const MAGIC: [u8; 6] = [0x6a, 0x24, 0xaa, 0x21, 0xa9, 0xed];
        // Witness commitment is optional if there are no transactions using SegWit in the block.
        if self.txdata.iter().all(|t| t.input.iter().all(|i| i.witness.is_empty())) {
            return true;
        }

        if self.txdata.is_empty() {
            return false;
        }

        let coinbase = &self.txdata[0];
        if !coinbase.is_coin_base() {
            return false;
        }

        // Commitment is in the last output that starts with magic bytes.
        if let Some(pos) = coinbase.output.iter()
            .rposition(|o| o.script_pubkey.len () >= 38 && o.script_pubkey[0..6] ==  MAGIC)
        {
            let commitment = WitnessCommitment::from_slice(&coinbase.output[pos].script_pubkey.as_bytes()[6..38]).unwrap();
            // Witness reserved value is in coinbase input witness.
            let witness_vec: Vec<_> = coinbase.input[0].witness.iter().collect();
            if witness_vec.len() == 1 && witness_vec[0].len() == 32 {
                if let Some(witness_root) = self.witness_root() {
                    return commitment == Self::compute_witness_commitment(&witness_root, witness_vec[0]);
                }
            }
        }

        false
    }

    /// Computes the transaction merkle root.
    pub fn compute_merkle_root(&self) -> Option<TxMerkleNode> {
        let hashes = self.txdata.iter().map(|obj| obj.txid().as_hash());
        bitcoin_merkle_root(hashes).map(|h| h.into())
    }

    /// Calculate the transaction merkle root.
    #[deprecated(since = "0.28.0", note = "Please use `block::compute_merkle_root` instead.")]
    pub fn merkle_root(&self) -> Option<TxMerkleNode> {
        self.compute_merkle_root()
    }

    /// Computes the witness commitment for the block's transaction list.
    pub fn compute_witness_commitment(witness_root: &WitnessMerkleNode, witness_reserved_value: &[u8]) -> WitnessCommitment {
        let mut encoder = WitnessCommitment::engine();
        witness_root.consensus_encode(&mut encoder).expect("engines don't error");
        encoder.input(witness_reserved_value);
        WitnessCommitment::from_engine(encoder)
    }

    /// Computes the merkle root of transactions hashed for witness.
    pub fn witness_root(&self) -> Option<WitnessMerkleNode> {
        let hashes = self.txdata.iter().enumerate().map(|(i, t)| {
            if i == 0 {
                // Replace the first hash with zeroes.
                Wtxid::default().as_hash()
            } else {
                t.wtxid().as_hash()
            }
        });
        bitcoin_merkle_root(hashes).map(|h| h.into())
    }

    /// base_size == size of header + size of encoded transaction count.
    fn base_size(&self) -> usize {
        self.header.get_size() + VarInt(self.txdata.len() as u64).len()
    }

    /// Returns the size of the block.
    #[deprecated(since = "0.28.0", note = "Please use `block::size` instead.")]
    pub fn get_size(&self) -> usize {
        self.size()
    }

    /// Returns the size of the block.
    ///
    /// size == size of header + size of encoded transaction count + total size of transactions.
    pub fn size(&self) -> usize {
        let txs_size: usize = self.txdata.iter().map(Transaction::size).sum();
        self.base_size() + txs_size
    }

    /// Returns the strippedsize of the block.
    #[deprecated(since = "0.28.0", note = "Please use `transaction::strippedsize` instead.")]
    pub fn get_strippedsize(&self) -> usize {
        self.strippedsize()
    }

    /// Returns the strippedsize of the block.
    pub fn strippedsize(&self) -> usize {
        let txs_size: usize = self.txdata.iter().map(Transaction::strippedsize).sum();
        self.base_size() + txs_size
    }

    /// Returns the weight of the block.
    #[deprecated(since = "0.28.0", note = "Please use `transaction::weight` instead.")]
    pub fn get_weight(&self) -> usize {
        self.weight()
    }

    /// Returns the weight of the block.
    pub fn weight(&self) -> usize {
        let base_weight = WITNESS_SCALE_FACTOR * self.base_size();
        let txs_weight: usize = self.txdata.iter().map(Transaction::weight).sum();
        base_weight + txs_weight
    }

    /// Returns the coinbase transaction, if one is present.
    pub fn coinbase(&self) -> Option<&Transaction> {
        self.txdata.first()
    }

    /// Returns the block height, as encoded in the coinbase transaction according to BIP34.
    pub fn bip34_block_height(&self) -> Result<u64, Bip34Error> {
        // Citing the spec:
        // Add height as the first item in the coinbase transaction's scriptSig,
        // and increase block version to 2. The format of the height is
        // "serialized CScript" -- first byte is number of bytes in the number
        // (will be 0x03 on main net for the next 150 or so years with 2^23-1
        // blocks), following bytes are little-endian representation of the
        // number (including a sign bit). Height is the height of the mined
        // block in the block chain, where the genesis block is height zero (0).

        if self.header.version < 2 {
            return Err(Bip34Error::Unsupported);
        }

        let cb = self.coinbase().ok_or(Bip34Error::NotPresent)?;
        let input = cb.input.first().ok_or(Bip34Error::NotPresent)?;
        let push = input.script_sig.instructions_minimal().next().ok_or(Bip34Error::NotPresent)?;
        match push.map_err(|_| Bip34Error::NotPresent)? {
            script::Instruction::PushBytes(b) if b.len() <= 8 => {
                // Expand the push to exactly 8 bytes (LE).
                let mut full = [0; 8];
                full[0..b.len()].copy_from_slice(b);
                Ok(util::endian::slice_to_u64_le(&full))
            }
            script::Instruction::PushBytes(b) if b.len() > 8 => {
                Err(Bip34Error::UnexpectedPush(b.to_vec()))
            }
            _ => Err(Bip34Error::NotPresent),
        }
    }
}

impl fmt::Debug for SimpleHeader {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.debug_struct("Header")
            //.field("block_hash", &self.block_hash())
            .field("version", &self.version)
            .field("prev_blockhash", &self.prev_blockhash)
            .field("merkle_root", &self.merkle_root)
            .field("time", &self.time)
            .field("bits", &self.bits)
            .field("nonce", &self.nonce)
            .finish()
    }
}

#[derive(PartialEq, Eq, Clone, Debug, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct MerkleBranch {
    pub hashes: Vec<BlockHash>,
    // Bitmask of which side of the merkle hash function the branch_hash element should go on.
    // Zero means it goes on the right, One means on the left.
    // It is equal to the index of the starting hash within the widest level
    // of the merkle tree for this merkle branch.
    pub side_mask: u32,
}
impl_consensus_encoding!(MerkleBranch, hashes, side_mask);

#[derive(PartialEq, Eq, Clone, Debug, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]

pub struct AuxPow {
    pub coinbase_tx: Transaction,
    pub block_hash: BlockHash,
    pub coinbase_branch: MerkleBranch,
    pub blockchain_branch: MerkleBranch,
    pub parent_block: SimpleHeader,
}

impl_consensus_encoding!(
    AuxPow,
    coinbase_tx,
    block_hash,
    coinbase_branch,
    blockchain_branch,
    parent_block
);
impl AuxPow {
    pub fn get_size(&self) -> usize {
        self.coinbase_tx.total_size()
            + 32
            + self.coinbase_branch.hashes.len() * 32
            + self.blockchain_branch.hashes.len() * 32
            + 80
    }
}

/// An error when looking up a BIP34 block height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Bip34Error {
    /// The block does not support BIP34 yet.
    Unsupported,
    /// No push was present where the BIP34 push was expected.
    NotPresent,
    /// The BIP34 push was larger than 8 bytes.
    UnexpectedPush(Vec<u8>),
}

impl fmt::Display for Bip34Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            Bip34Error::Unsupported => write!(f, "block doesn't support BIP34"),
            Bip34Error::NotPresent => write!(f, "BIP34 push not present in block's coinbase"),
            Bip34Error::UnexpectedPush(ref p) => {
                write!(f, "unexpected byte push of > 8 bytes: {:?}", p)
            }
        }
    }
}

#[cfg(feature = "std")]
impl ::std::error::Error for Bip34Error {}

#[cfg(test)]
mod tests {
    use hashes::hex::FromHex;

    use blockdata::block::{Block, BlockHeader};
    use consensus::encode::{deserialize, serialize};
    use util::uint::Uint256;
    use util::Error::{BlockBadTarget, BlockBadProofOfWork};
    use network::constants::Network;

    #[test]
    fn test_coinbase_and_bip34() {
        // testnet block 100,000
        let block_hex = "0200000035ab154183570282ce9afc0b494c9fc6a3cfea05aa8c1add2ecc56490000000038ba3d78e4500a5a7570dbe61960398add4410d278b21cd9708e6d9743f374d544fc055227f1001c29c1ea3b0101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff3703a08601000427f1001c046a510100522cfabe6d6d0000000000000000000068692066726f6d20706f6f6c7365727665726aac1eeeed88ffffffff0100f2052a010000001976a914912e2b234f941f30b18afbb4fa46171214bf66c888ac00000000";
        let block: Block = deserialize(&Vec::<u8>::from_hex(block_hex).unwrap()).unwrap();

        let cb_txid = "d574f343976d8e70d91cb278d21044dd8a396019e6db70755a0a50e4783dba38";
        assert_eq!(block.coinbase().unwrap().txid().to_string(), cb_txid);

        assert_eq!(block.bip34_block_height(), Ok(100_000));


        // block with 9-byte bip34 push
        let bad_hex = "0200000035ab154183570282ce9afc0b494c9fc6a3cfea05aa8c1add2ecc56490000000038ba3d78e4500a5a7570dbe61960398add4410d278b21cd9708e6d9743f374d544fc055227f1001c29c1ea3b0101000000010000000000000000000000000000000000000000000000000000000000000000ffffffff3d09a08601112233445566000427f1001c046a510100522cfabe6d6d0000000000000000000068692066726f6d20706f6f6c7365727665726aac1eeeed88ffffffff0100f2052a010000001976a914912e2b234f941f30b18afbb4fa46171214bf66c888ac00000000";
        let bad: Block = deserialize(&Vec::<u8>::from_hex(bad_hex).unwrap()).unwrap();

        let push = Vec::<u8>::from_hex("a08601112233445566").unwrap();
        assert_eq!(bad.bip34_block_height(), Err(super::Bip34Error::UnexpectedPush(push)));
    }

    #[test]
    fn block_test() {
        // Mainnet block 00000000b0c5a240b2a61d2e75692224efd4cbecdf6eaf4cc2cf477ca7c270e7
        let some_block = Vec::from_hex("010000004ddccd549d28f385ab457e98d1b11ce80bfea2c5ab93015ade4973e400000000bf4473e53794beae34e64fccc471dace6ae544180816f89591894e0f417a914cd74d6e49ffff001d323b3a7b0201000000010000000000000000000000000000000000000000000000000000000000000000ffffffff0804ffff001d026e04ffffffff0100f2052a0100000043410446ef0102d1ec5240f0d061a4246c1bdef63fc3dbab7733052fbbf0ecd8f41fc26bf049ebb4f9527f374280259e7cfa99c48b0e3f39c51347a19a5819651503a5ac00000000010000000321f75f3139a013f50f315b23b0c9a2b6eac31e2bec98e5891c924664889942260000000049483045022100cb2c6b346a978ab8c61b18b5e9397755cbd17d6eb2fe0083ef32e067fa6c785a02206ce44e613f31d9a6b0517e46f3db1576e9812cc98d159bfdaf759a5014081b5c01ffffffff79cda0945903627c3da1f85fc95d0b8ee3e76ae0cfdc9a65d09744b1f8fc85430000000049483045022047957cdd957cfd0becd642f6b84d82f49b6cb4c51a91f49246908af7c3cfdf4a022100e96b46621f1bffcf5ea5982f88cef651e9354f5791602369bf5a82a6cd61a62501fffffffffe09f5fe3ffbf5ee97a54eb5e5069e9da6b4856ee86fc52938c2f979b0f38e82000000004847304402204165be9a4cbab8049e1af9723b96199bfd3e85f44c6b4c0177e3962686b26073022028f638da23fc003760861ad481ead4099312c60030d4cb57820ce4d33812a5ce01ffffffff01009d966b01000000434104ea1feff861b51fe3f5f8a3b12d0f4712db80e919548a80839fc47c6a21e66d957e9c5d8cd108c7a2d2324bad71f9904ac0ae7336507d785b17a2c115e427a32fac00000000").unwrap();
        let cutoff_block = Vec::from_hex("010000004ddccd549d28f385ab457e98d1b11ce80bfea2c5ab93015ade4973e400000000bf4473e53794beae34e64fccc471dace6ae544180816f89591894e0f417a914cd74d6e49ffff001d323b3a7b0201000000010000000000000000000000000000000000000000000000000000000000000000ffffffff0804ffff001d026e04ffffffff0100f2052a0100000043410446ef0102d1ec5240f0d061a4246c1bdef63fc3dbab7733052fbbf0ecd8f41fc26bf049ebb4f9527f374280259e7cfa99c48b0e3f39c51347a19a5819651503a5ac00000000010000000321f75f3139a013f50f315b23b0c9a2b6eac31e2bec98e5891c924664889942260000000049483045022100cb2c6b346a978ab8c61b18b5e9397755cbd17d6eb2fe0083ef32e067fa6c785a02206ce44e613f31d9a6b0517e46f3db1576e9812cc98d159bfdaf759a5014081b5c01ffffffff79cda0945903627c3da1f85fc95d0b8ee3e76ae0cfdc9a65d09744b1f8fc85430000000049483045022047957cdd957cfd0becd642f6b84d82f49b6cb4c51a91f49246908af7c3cfdf4a022100e96b46621f1bffcf5ea5982f88cef651e9354f5791602369bf5a82a6cd61a62501fffffffffe09f5fe3ffbf5ee97a54eb5e5069e9da6b4856ee86fc52938c2f979b0f38e82000000004847304402204165be9a4cbab8049e1af9723b96199bfd3e85f44c6b4c0177e3962686b26073022028f638da23fc003760861ad481ead4099312c60030d4cb57820ce4d33812a5ce01ffffffff01009d966b01000000434104ea1feff861b51fe3f5f8a3b12d0f4712db80e919548a80839fc47c6a21e66d957e9c5d8cd108c7a2d2324bad71f9904ac0ae7336507d785b17a2c115e427a32fac").unwrap();

        let prevhash = Vec::from_hex("4ddccd549d28f385ab457e98d1b11ce80bfea2c5ab93015ade4973e400000000").unwrap();
        let merkle = Vec::from_hex("bf4473e53794beae34e64fccc471dace6ae544180816f89591894e0f417a914c").unwrap();
        let work = Uint256([0x100010001u64, 0, 0, 0]);

        let decode: Result<Block, _> = deserialize(&some_block);
        let bad_decode: Result<Block, _> = deserialize(&cutoff_block);

        assert!(decode.is_ok());
        assert!(bad_decode.is_err());
        let real_decode = decode.unwrap();
        assert_eq!(real_decode.header.version, 1);
        assert_eq!(serialize(&real_decode.header.prev_blockhash), prevhash);
        assert_eq!(real_decode.header.merkle_root, real_decode.compute_merkle_root().unwrap());
        assert_eq!(serialize(&real_decode.header.merkle_root), merkle);
        assert_eq!(real_decode.header.time, 1231965655);
        assert_eq!(real_decode.header.bits, 486604799);
        assert_eq!(real_decode.header.nonce, 2067413810);
        assert_eq!(real_decode.header.work(), work);
        assert_eq!(real_decode.header.validate_pow(&real_decode.header.target()).unwrap(), real_decode.block_hash());
        assert_eq!(real_decode.header.difficulty(Network::Bitcoin), 1);
        // [test] TODO: check the transaction data

        assert_eq!(real_decode.size(), some_block.len());
        assert_eq!(real_decode.strippedsize(), some_block.len());
        assert_eq!(real_decode.weight(), some_block.len() * 4);

        // should be also ok for a non-witness block as commitment is optional in that case
        assert!(real_decode.check_witness_commitment());

        assert_eq!(serialize(&real_decode), some_block);
    }

    // Check testnet block 000000000000045e0b1660b6445b5e5c5ab63c9a4f956be7e1e69be04fa4497b
    #[test]
    fn segwit_block_test() {
        let segwit_block = include_bytes!("../../test_data/testnet_block_000000000000045e0b1660b6445b5e5c5ab63c9a4f956be7e1e69be04fa4497b.raw").to_vec();

        let decode: Result<Block, _> = deserialize(&segwit_block);

        let prevhash = Vec::from_hex("2aa2f2ca794ccbd40c16e2f3333f6b8b683f9e7179b2c4d74906000000000000").unwrap();
        let merkle = Vec::from_hex("10bc26e70a2f672ad420a6153dd0c28b40a6002c55531bfc99bf8994a8e8f67e").unwrap();
        let work = Uint256([0x257c3becdacc64u64, 0, 0, 0]);

        assert!(decode.is_ok());
        let real_decode = decode.unwrap();
        assert_eq!(real_decode.header.version, 0x20000000);  // VERSIONBITS but no bits set
        assert_eq!(serialize(&real_decode.header.prev_blockhash), prevhash);
        assert_eq!(serialize(&real_decode.header.merkle_root), merkle);
        assert_eq!(real_decode.header.merkle_root, real_decode.compute_merkle_root().unwrap());
        assert_eq!(real_decode.header.time, 1472004949);
        assert_eq!(real_decode.header.bits, 0x1a06d450);
        assert_eq!(real_decode.header.nonce, 1879759182);
        assert_eq!(real_decode.header.work(), work);
        assert_eq!(real_decode.header.validate_pow(&real_decode.header.target()).unwrap(), real_decode.block_hash());
        assert_eq!(real_decode.header.difficulty(Network::Testnet), 2456598);
        // [test] TODO: check the transaction data

        assert_eq!(real_decode.size(), segwit_block.len());
        assert_eq!(real_decode.strippedsize(), 4283);
        assert_eq!(real_decode.weight(), 17168);

        assert!(real_decode.check_witness_commitment());

        assert_eq!(serialize(&real_decode), segwit_block);
    }

    #[test]
    fn block_version_test() {
        let block = Vec::from_hex("ffffff7f0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").unwrap();
        let decode: Result<Block, _> = deserialize(&block);
        assert!(decode.is_ok());
        let real_decode = decode.unwrap();
        assert_eq!(real_decode.header.version, 2147483647);

        let block2 = Vec::from_hex("000000800000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000").unwrap();
        let decode2: Result<Block, _> = deserialize(&block2);
        assert!(decode2.is_ok());
        let real_decode2 = decode2.unwrap();
        assert_eq!(real_decode2.header.version, -2147483648);
    }

    #[test]
    fn validate_pow_test() {
        let some_header = Vec::from_hex("010000004ddccd549d28f385ab457e98d1b11ce80bfea2c5ab93015ade4973e400000000bf4473e53794beae34e64fccc471dace6ae544180816f89591894e0f417a914cd74d6e49ffff001d323b3a7b").unwrap();
        let some_header: BlockHeader = deserialize(&some_header).expect("Can't deserialize correct block header");
        assert_eq!(some_header.validate_pow(&some_header.target()).unwrap(), some_header.block_hash());

        // test with zero target
        match some_header.validate_pow(&Uint256::default()) {
            Err(BlockBadTarget) => (),
            _ => assert!(false)
        }

        // test with modified header
        let mut invalid_header: BlockHeader = some_header.clone();
        invalid_header.version = invalid_header.version + 1;
        match invalid_header.validate_pow(&invalid_header.target()) {
            Err(BlockBadProofOfWork) => (),
            _ => assert!(false)
        }
    }

    #[test]
    fn compact_roundrtip_test() {
        let some_header = Vec::from_hex("010000004ddccd549d28f385ab457e98d1b11ce80bfea2c5ab93015ade4973e400000000bf4473e53794beae34e64fccc471dace6ae544180816f89591894e0f417a914cd74d6e49ffff001d323b3a7b").unwrap();

        let header: BlockHeader = deserialize(&some_header).expect("Can't deserialize correct block header");

        assert_eq!(header.bits, BlockHeader::compact_target_from_u256(&header.target()));
    }
}

#[cfg(all(test, feature = "unstable"))]
mod benches {
    use super::Block;
    use EmptyWrite;
    use consensus::{deserialize, Encodable};
    use test::{black_box, Bencher};
    use network::stream_reader::StreamReader;

    #[bench]
    #[allow(deprecated)]
    pub fn bench_stream_reader(bh: &mut Bencher) {
        let big_block = include_bytes!("../../test_data/mainnet_block_000000000000000000000c835b2adcaedc20fdf6ee440009c249452c726dafae.raw");
        assert_eq!(big_block.len(), 1_381_836);
        let big_block = black_box(big_block);

        bh.iter(|| {
            let mut reader = StreamReader::new(&big_block[..], None);
            let block: Block = reader.read_next().unwrap();
            black_box(&block);
        });
    }

    #[bench]
    pub fn bench_block_serialize(bh: &mut Bencher) {
        let raw_block = include_bytes!("../../test_data/mainnet_block_000000000000000000000c835b2adcaedc20fdf6ee440009c249452c726dafae.raw");

        let block: Block = deserialize(&raw_block[..]).unwrap();

        let mut data = Vec::with_capacity(raw_block.len());

        bh.iter(|| {
            let result = block.consensus_encode(&mut data);
            black_box(&result);
            data.clear();
        });
    }

    #[bench]
    pub fn bench_block_serialize_logic(bh: &mut Bencher) {
        let raw_block = include_bytes!("../../test_data/mainnet_block_000000000000000000000c835b2adcaedc20fdf6ee440009c249452c726dafae.raw");

        let block: Block = deserialize(&raw_block[..]).unwrap();

        bh.iter(|| {
            let size = block.consensus_encode(&mut EmptyWrite);
            black_box(&size);
        });
    }

    #[bench]
    pub fn bench_block_deserialize(bh: &mut Bencher) {
        let raw_block = include_bytes!("../../test_data/mainnet_block_000000000000000000000c835b2adcaedc20fdf6ee440009c249452c726dafae.raw");

        bh.iter(|| {
            let block: Block = deserialize(&raw_block[..]).unwrap();
            black_box(&block);
        });
    }
}
