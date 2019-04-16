// -*- mode: rust; -*-
//
// This file is part of twopac.
// Copyright © 2019 Galois, Inc.
// See LICENSE for licensing information.

use crate::errors::Error;
use fancy_garbling::{Evaluator as Ev, Fancy, Wire};
use ocelot::ot::Receiver as OtReceiver;
use rand::{CryptoRng, RngCore};
use scuttlebutt::Block;
use std::cell::RefCell;
use std::fmt::Debug;
use std::io::{Read, Write};
use std::rc::Rc;

/// Semi-honest evaluator.
pub struct Evaluator<R: Read + Debug, W: Write + Debug, RNG: CryptoRng + RngCore, OT: OtReceiver> {
    evaluator: Ev<R>,
    reader: Rc<RefCell<R>>,
    writer: Rc<RefCell<W>>,
    inputs: Vec<u16>,
    ot: OT,
    rng: RNG,
}

impl<
        R: Read + Send + Debug + 'static,
        W: Write + Send + Debug,
        RNG: CryptoRng + RngCore,
        OT: OtReceiver<Msg = Block>,
    > Evaluator<R, W, RNG, OT>
{
    /// Make a new `Evaluator`.
    pub fn new(mut reader: R, mut writer: W, inputs: &[u16], mut rng: RNG) -> Result<Self, Error> {
        let ot = OT::init(&mut reader, &mut writer, &mut rng)?;
        let reader = Rc::new(RefCell::new(reader));
        let writer = Rc::new(RefCell::new(writer));
        let evaluator = Ev::new(reader.clone());
        let inputs = inputs.to_vec();
        Ok(Evaluator {
            evaluator,
            reader,
            writer,
            inputs,
            ot,
            rng,
        })
    }

    /// Decode the output post-evaluation.
    pub fn decode_output(&self) -> Vec<u16> {
        self.evaluator.decode_output()
    }

    fn run_ot(&mut self, inputs: &[bool]) -> Result<Vec<Block>, Error> {
        self.ot
            .receive(
                &mut *self.reader.borrow_mut(),
                &mut *self.writer.borrow_mut(),
                &inputs,
                &mut self.rng,
            )
            .map_err(Error::from)
    }
}

fn combine(wires: &[Block], q: u16) -> Wire {
    wires
        .into_iter()
        .enumerate()
        .fold(Wire::zero(q), |acc, (i, w)| {
            let w = Wire::from_block(*w, q);
            acc.plus(&w.cmul(1 << i))
        })
}

impl<
        R: Read + Send + Debug + 'static,
        W: Write + Send + Debug,
        RNG: CryptoRng + RngCore,
        OT: OtReceiver<Msg = Block>,
    > Fancy for Evaluator<R, W, RNG, OT>
{
    type Item = Wire;
    type Error = Error;

    #[inline]
    fn garbler_input(&mut self, q: u16, opt_x: Option<u16>) -> Result<Self::Item, Self::Error> {
        self.evaluator
            .garbler_input(q, opt_x)
            .map_err(Self::Error::from)
    }
    #[inline]
    fn evaluator_input(&mut self, q: u16) -> Result<Self::Item, Self::Error> {
        let wires = self.evaluator_inputs(&[q])?;
        Ok(wires[0].clone())
    }
    #[inline]
    fn evaluator_inputs(&mut self, qs: &[u16]) -> Result<Vec<Self::Item>, Self::Error> {
        let lens = qs
            .into_iter()
            .map(|q| (*q as f32).log(2.0).ceil() as usize)
            .collect::<Vec<usize>>();
        let mut bs = Vec::with_capacity(lens.iter().sum());
        for len in lens.iter() {
            let input = self.inputs.remove(0);
            for b in (0..*len).into_iter().map(|i| input & (1 << i) != 0) {
                bs.push(b);
            }
        }
        let wires = self.run_ot(&bs)?;
        let mut start = 0;
        Ok(lens
            .into_iter()
            .zip(qs.into_iter())
            .map(|(len, q)| {
                let range = start..start + len;
                let chunk = &wires[range];
                start = start + len;
                combine(chunk, *q)
            })
            .collect::<Vec<Wire>>())
    }
    #[inline]
    fn constant(&mut self, x: u16, q: u16) -> Result<Self::Item, Self::Error> {
        self.evaluator.constant(x, q).map_err(Self::Error::from)
    }
    #[inline]
    fn add(&mut self, x: &Wire, y: &Wire) -> Result<Self::Item, Self::Error> {
        self.evaluator.add(&x, &y).map_err(Self::Error::from)
    }
    #[inline]
    fn sub(&mut self, x: &Wire, y: &Wire) -> Result<Self::Item, Self::Error> {
        self.evaluator.sub(&x, &y).map_err(Self::Error::from)
    }
    #[inline]
    fn cmul(&mut self, x: &Wire, c: u16) -> Result<Self::Item, Self::Error> {
        self.evaluator.cmul(&x, c).map_err(Self::Error::from)
    }
    #[inline]
    fn mul(&mut self, x: &Wire, y: &Wire) -> Result<Self::Item, Self::Error> {
        self.evaluator.mul(&x, &y).map_err(Self::Error::from)
    }
    #[inline]
    fn proj(&mut self, x: &Wire, q: u16, tt: Option<Vec<u16>>) -> Result<Self::Item, Self::Error> {
        self.evaluator.proj(&x, q, tt).map_err(Self::Error::from)
    }
    #[inline]
    fn output(&mut self, x: &Wire) -> Result<(), Self::Error> {
        self.evaluator.output(&x).map_err(Self::Error::from)
    }
}
