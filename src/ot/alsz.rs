// -*- mode: rust; -*-
//
// This file is part of ocelot.
// Copyright © 2019 Galois, Inc.
// See LICENSE for licensing information.

use crate::hash_aes::AesHash;
use crate::rand_aes::AesRng;
use crate::stream;
use crate::utils;
use crate::{Block, BlockObliviousTransfer, SemiHonest};
use arrayref::array_ref;
use failure::Error;
use std::io::{BufReader, BufWriter, ErrorKind, Read, Write};
use std::marker::PhantomData;

/// Implementation of the Asharov-Lindell-Schneider-Zohner oblivious transfer
/// extension protocol (cf. <https://eprint.iacr.org/2016/602>, Protocol 4).
pub struct AlszOT<S: Read + Write + Send + Sync, OT: BlockObliviousTransfer<S> + SemiHonest> {
    _s: PhantomData<S>,
    ot: OT,
    rng: AesRng,
}

impl<S: Read + Write + Send + Sync, OT: BlockObliviousTransfer<S> + SemiHonest>
    BlockObliviousTransfer<S> for AlszOT<S, OT>
{
    fn new() -> Self {
        let ot = OT::new();
        let rng = AesRng::new(&rand::random::<Block>());
        Self {
            _s: PhantomData::<S>,
            ot,
            rng,
        }
    }

    fn send(
        &mut self,
        reader: &mut BufReader<S>,
        mut writer: &mut BufWriter<S>,
        inputs: &[(Block, Block)],
    ) -> Result<(), Error> {
        let m = inputs.len();
        if m % 8 != 0 {
            return Err(Error::from(std::io::Error::new(
                ErrorKind::InvalidInput,
                "Number of inputs must be divisible by 8",
            )));
        }
        if m <= 128 {
            // Just do normal OT
            return self.ot.send(reader, writer, inputs);
        }
        let (nrows, ncols) = (128, m);
        let hash = AesHash::new(&[0u8; 16]); // XXX IV should be chosen at random

        let mut s_ = vec![0u8; nrows / 8];
        self.rng.random(&mut s_);
        let s = utils::u8vec_to_boolvec(&s_);
        let ks = self.ot.receive(reader, writer, &s)?;
        let rngs = ks.into_iter().map(|k| AesRng::new(&k));
        let mut qs = vec![0u8; nrows * ncols / 8];
        let mut u = vec![0u8; ncols / 8];
        for (j, (b, rng)) in s.into_iter().zip(rngs).enumerate() {
            let range = j * ncols / 8..(j + 1) * ncols / 8;
            let mut q = &mut qs[range];
            stream::read_bytes_inplace(reader, &mut u)?;
            if !b {
                std::mem::replace(&mut u, vec![0u8; ncols / 8]);
            };
            rng.random(&mut q);
            utils::xor_inplace(&mut q, &u);
        }
        let mut qs = utils::transpose(&qs, nrows, ncols);
        for (j, input) in inputs.iter().enumerate() {
            let range = j * nrows / 8..(j + 1) * nrows / 8;
            let mut q = &mut qs[range];
            let y0 = utils::xor_block(&hash.cr_hash(j, array_ref![q, 0, 16]), &input.0);
            utils::xor_inplace(&mut q, &s_);
            let y1 = utils::xor_block(&hash.cr_hash(j, array_ref![q, 0, 16]), &input.1);
            stream::write_block(&mut writer, &y0)?;
            stream::write_block(&mut writer, &y1)?;
        }
        Ok(())
    }

    fn receive(
        &mut self,
        mut reader: &mut BufReader<S>,
        mut writer: &mut BufWriter<S>,
        inputs: &[bool],
    ) -> Result<Vec<Block>, Error> {
        let m = inputs.len();
        if m <= 128 {
            // Just do normal OT
            return self.ot.receive(reader, writer, inputs);
        }
        let (nrows, ncols) = (128, m);
        let hash = AesHash::new(&[0u8; 16]); // XXX IV should be chosen at random
        let mut ks = Vec::with_capacity(nrows);
        for _ in 0..nrows {
            let mut k0 = [0u8; 16];
            let mut k1 = [0u8; 16];
            self.rng.random(&mut k0);
            self.rng.random(&mut k1);
            ks.push((k0, k1));
        }
        self.ot.send(reader, writer, &ks)?;
        let rngs = ks
            .into_iter()
            .map(|(k0, k1)| (AesRng::new(&k0), AesRng::new(&k1)))
            .collect::<Vec<(AesRng, AesRng)>>();
        let r = utils::boolvec_to_u8vec(inputs);
        let mut ts = vec![0u8; nrows * ncols / 8];
        let mut g = vec![0u8; ncols / 8];
        for (j, (rng0, rng1)) in rngs.into_iter().enumerate() {
            let range = j * ncols / 8..(j + 1) * ncols / 8;
            let mut t = &mut ts[range];
            rng0.random(&mut t);
            rng1.random(&mut g);
            utils::xor_inplace(&mut g, &t);
            utils::xor_inplace(&mut g, &r);
            stream::write_bytes(&mut writer, &g)?;
            writer.flush()?;
        }
        let ts = utils::transpose(&ts, nrows, ncols);
        let mut out = Vec::with_capacity(ncols);
        for (j, b) in inputs.iter().enumerate() {
            let range = j * nrows / 8..(j + 1) * nrows / 8;
            let t = &ts[range];
            let y0 = stream::read_block(&mut reader)?;
            let y1 = stream::read_block(&mut reader)?;
            let y = if *b { y1 } else { y0 };
            let y = utils::xor_block(&y, &hash.cr_hash(j, array_ref![t, 0, 16]));
            out.push(y);
        }
        Ok(out)
    }
}

impl<S: Read + Write + Send + Sync, OT: BlockObliviousTransfer<S> + SemiHonest> SemiHonest
    for AlszOT<S, OT>
{
}

#[cfg(test)]
mod tests {
    extern crate test;
    use super::*;
    use crate::*;
    use itertools::izip;
    use std::os::unix::net::UnixStream;

    const T: usize = 1 << 12;

    fn rand_block_vec(size: usize) -> Vec<Block> {
        (0..size).map(|_| rand::random::<Block>()).collect()
    }

    fn rand_bool_vec(size: usize) -> Vec<bool> {
        (0..size).map(|_| rand::random::<bool>()).collect()
    }

    fn test_ot<OT: BlockObliviousTransfer<UnixStream> + SemiHonest>() {
        let m0s = rand_block_vec(T);
        let m1s = rand_block_vec(T);
        let bs = rand_bool_vec(T);
        let m0s_ = m0s.clone();
        let m1s_ = m1s.clone();
        let bs_ = bs.clone();
        let (sender, receiver) = UnixStream::pair().unwrap();
        let handle = std::thread::spawn(move || {
            let mut otext = AlszOT::<UnixStream, OT>::new();
            let mut reader = BufReader::new(sender.try_clone().unwrap());
            let mut writer = BufWriter::new(sender);
            let ms = m0s
                .into_iter()
                .zip(m1s.into_iter())
                .collect::<Vec<(Block, Block)>>();
            otext.send(&mut reader, &mut writer, &ms).unwrap();
        });
        let mut otext = AlszOT::<UnixStream, OT>::new();
        let mut reader = BufReader::new(receiver.try_clone().unwrap());
        let mut writer = BufWriter::new(receiver);
        let results = otext.receive(&mut reader, &mut writer, &bs).unwrap();
        for (b, result, m0, m1) in izip!(bs_, results, m0s_, m1s_) {
            assert_eq!(result, if b { m1 } else { m0 })
        }
        handle.join().unwrap();
    }

    #[test]
    fn test() {
        test_ot::<ChouOrlandiOT<UnixStream>>();
    }
}
