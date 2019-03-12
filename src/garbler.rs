// -*- mode: rust; -*-
//
// This file is part of twopac.
// Copyright © 2019 Galois, Inc.
// See LICENSE for licensing information.

use crate::comm;
use crate::errors::Error;
use fancy_garbling::{Fancy, Garbler as Gb, Message, SyncIndex, Wire};
use ocelot::ObliviousTransferSender;
use rand::{CryptoRng, RngCore};
use scuttlebutt::Block;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

pub struct Garbler<
    R: Read + Send,
    W: Write + Send,
    RNG: CryptoRng + RngCore,
    OT: ObliviousTransferSender,
> {
    garbler: Gb,
    reader: Arc<Mutex<R>>,
    writer: Arc<Mutex<W>>,
    ot: Arc<Mutex<OT>>,
    rng: Arc<Mutex<RNG>>,
}

impl<
        R: Read + Send,
        W: Write + Send + 'static,
        RNG: CryptoRng + RngCore,
        OT: ObliviousTransferSender<Msg = Block>,
    > Garbler<R, W, RNG, OT>
{
    pub fn new(mut reader: R, mut writer: W, inputs: &[u16], mut rng: RNG) -> Result<Self, Error> {
        let ot = OT::init(&mut reader, &mut writer, &mut rng)?;
        let mut inputs = inputs.to_vec().into_iter();
        let reader = Arc::new(Mutex::new(reader));
        let writer = Arc::new(Mutex::new(writer));
        let writer_ = writer.clone();
        let callback = move |idx: Option<SyncIndex>, msg| {
            let m = match msg {
                Message::UnencodedGarblerInput { zero, delta } => {
                    let input = inputs.next().unwrap();
                    Message::GarblerInput(zero.plus(&delta.cmul(input)))
                }
                Message::UnencodedEvaluatorInput { zero: _, delta: _ } => {
                    panic!("There should not be an UnencodedEvaluatorInput message in the garbler");
                }
                Message::EvaluatorInput(_) => {
                    panic!("There should not be an EvaluatorInput message in the garbler");
                }
                m => m,
            };
            let mut writer = writer_.lock().unwrap();
            match idx {
                Some(i) => comm::send(&mut *writer, &[i]).expect("Unable to send index"),
                None => comm::send(&mut *writer, &[0xFF]).expect("Unable to send index"),
            }
            comm::send(&mut *writer, &m.to_bytes()).expect("Unable to send message");
        };
        let garbler = Gb::new(callback);
        let ot = Arc::new(Mutex::new(ot));
        let rng = Arc::new(Mutex::new(rng));
        Ok(Garbler {
            garbler,
            reader,
            writer,
            ot,
            rng,
        })
    }

    fn run_ot(&self, inputs: &[(Block, Block)]) {
        let mut ot = self.ot.lock().unwrap();
        let mut reader = self.reader.lock().unwrap();
        let mut writer = self.writer.lock().unwrap();
        let mut rng = self.rng.lock().unwrap();
        ot.send(&mut *reader, &mut *writer, inputs, &mut *rng)
            .unwrap() // XXX: remove unwrap
    }
}

fn _evaluator_input(delta: &Wire, q: u16) -> (Wire, Vec<(Block, Block)>) {
    let len = (q as f32).log(2.0).ceil() as u16;
    let mut wire = Wire::zero(q);
    let inputs = (0..len)
        .into_iter()
        .map(|i| {
            let zero = Wire::rand(&mut rand::thread_rng(), q);
            let one = zero.plus(&delta);
            wire = wire.plus(&zero.cmul(1 << i));
            (super::wire_to_block(zero), super::wire_to_block(one))
        })
        .collect::<Vec<(Block, Block)>>();
    (wire, inputs)
}

impl<
        R: Read + Send,
        W: Write + Send + 'static,
        RNG: CryptoRng + RngCore,
        OT: ObliviousTransferSender<Msg = Block>,
    > Fancy for Garbler<R, W, RNG, OT>
{
    type Item = Wire;

    fn garbler_input(&self, ix: Option<SyncIndex>, q: u16, opt_x: Option<u16>) -> Wire {
        self.garbler.garbler_input(ix, q, opt_x)
    }

    fn evaluator_input(&self, _ix: Option<SyncIndex>, q: u16) -> Wire {
        let delta = self.garbler.delta(q);
        let (wire, inputs) = _evaluator_input(&delta, q);
        self.run_ot(&inputs);
        wire
    }

    fn evaluator_inputs(&self, _ix: Option<SyncIndex>, qs: &[u16]) -> Vec<Wire> {
        let n = qs.len();
        let lens = qs.into_iter().map(|q| (*q as f32).log(2.0).ceil() as usize);
        let mut wires = Vec::with_capacity(n);
        let mut inputs = Vec::with_capacity(lens.sum());
        for q in qs.into_iter() {
            let delta = self.garbler.delta(*q);
            let (wire, input) = _evaluator_input(&delta, *q);
            wires.push(wire);
            for i in input.into_iter() {
                inputs.push(i);
            }
        }
        self.run_ot(&inputs);
        wires
    }

    fn constant(&self, ix: Option<SyncIndex>, x: u16, q: u16) -> Wire {
        self.garbler.constant(ix, x, q)
    }

    fn add(&self, x: &Wire, y: &Wire) -> Wire {
        self.garbler.add(x, y)
    }

    fn sub(&self, x: &Wire, y: &Wire) -> Wire {
        self.garbler.sub(x, y)
    }

    fn cmul(&self, x: &Wire, c: u16) -> Wire {
        self.garbler.cmul(x, c)
    }

    fn mul(&self, ix: Option<SyncIndex>, x: &Wire, y: &Wire) -> Wire {
        self.garbler.mul(ix, x, y)
    }

    fn proj(&self, ix: Option<SyncIndex>, x: &Wire, q: u16, tt: Option<Vec<u16>>) -> Wire {
        self.garbler.proj(ix, x, q, tt)
    }

    fn output(&self, ix: Option<SyncIndex>, x: &Wire) {
        self.garbler.output(ix, x)
    }

    fn begin_sync(&self, n: SyncIndex) {
        self.garbler.begin_sync(n)
    }

    fn finish_index(&self, ix: SyncIndex) {
        self.garbler.finish_index(ix)
    }
}
