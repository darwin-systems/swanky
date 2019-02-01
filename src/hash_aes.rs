// -*- mode: rust; -*-
//
// This file is part of ocelot.
// Copyright © 2019 Galois, Inc.
// See LICENSE for licensing information.

//! Implementations of correlation-robust hash functions (and their variants)
//! based on fixed-key AES.

use crate::aes::Aes128;
use crate::utils;
use crate::Block;
use core::arch::x86_64::*;

/// AES-based correlation-robust hash function.
pub struct AesHash {
    aes: Aes128,
}

impl AesHash {
    #[inline(always)]
    pub fn new(key: &Block) -> Self {
        let aes = Aes128::new(key);
        AesHash { aes }
    }

    /// Correlation robust hash function for 128-bit inputs (cf.
    /// <https://eprint.iacr.org/2019/074>, §7.2).
    ///
    /// The function computes `π(x) ⊕ x`, where `π = AES(K, ·)` for some fixed
    /// key `K`.
    #[inline(always)]
    pub fn cr_hash(&self, _i: usize, x: &Block) -> Block {
        let y = self.aes.encrypt_u8(&x);
        utils::xor_block(&x, &y)
    }

    /// Circular correlation robust hash function (cf.
    /// <https://eprint.iacr.org/2019/074>, §7.3).
    ///
    /// The function computes `H(σ(x))`, where `H` is a correlation robust hash
    /// function and `σ(x₀ || x₁) = (x₀ ⊕ x₁) || x₁`.
    #[inline(always)]
    pub fn ccr_hash(&self, _i: usize, x: &Block) -> Block {
        unsafe {
            let x = _mm_xor_si128(
                _mm_shuffle_epi32(utils::block_to_m128i(x), 78),
                _mm_and_si128(
                    utils::block_to_m128i(x),
                    _mm_set_epi64(_mm_set1_pi8(0xF), _mm_setzero_si64()),
                ),
            );
            let x = utils::m128i_to_block(x);
            let y = self.aes.encrypt_u8(&x);
            utils::xor_block(&x, &y)
        }
    }
}
