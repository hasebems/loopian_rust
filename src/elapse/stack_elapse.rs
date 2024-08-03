//  Created by Hasebe Masahiko on 2023/01/22.
//  Copyright (c) 2023 Hasebe Masahiko.
//  Released under the MIT license
//  https://opensource.org/licenses/mit-license.php
//
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};
use std::vec::Vec;

use super::elapse::*;
use super::elapse_damper::DamperPart;
use super::elapse_flow::Flow;
use super::elapse_loop::{CompositionLoop, PhraseLoop};
use super::elapse_part::Part;
use super::miditx::MidiTx;
use super::tickgen::{CrntMsrTick, TickGen};
use crate::lpnlib::{ElpsMsg::*, *};
use crate::midirx::midirx::MidiRx;

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
pub enum SameKeyState {
    MORE,    //  まだある
    LAST,    //  これが最後
    NOTHING, //  もうない
}

//*******************************************************************
//          Elapse Stack Struct
//*******************************************************************
//  ElapseStack の責務
//  1. Elapse Object の生成と集約
//  2. Timing/Tempo の生成とtick管理
//  3. MIDI Out の生成と管理
pub struct ElapseStack {
    ui_hndr: mpsc::Sender<String>,
    rx_hndr: mpsc::Receiver<ElpsMsg>,
    tx_ctrl: mpsc::Sender<ElpsMsg>,
    mdx: MidiTx,

    crnt_time: Instant,
    bpm_stock: i16,
    beat_stock: Beat,

    during_play: bool,
    display_time: Instant,
    tg: TickGen,
    part_vec: Vec<Rc<RefCell<Part>>>, // Part Instance が繋がれた Vec
    _damper_part: Rc<RefCell<DamperPart>>,
    elapse_vec: Vec<Rc<RefCell<dyn Elapse>>>, // dyn Elapse Instance が繋がれた Vec
    key_map: [i32; (MAX_NOTE_NUMBER - MIN_NOTE_NUMBER + 1) as usize],
    limit_for_deb: i32,
}
//*******************************************************************
//          Public Method for Elapse Stack Struct
//*******************************************************************
fn gen_midirx_thread() -> (Receiver<ElpsMsg>, Sender<ElpsMsg>) {
    //  create new thread & channel
    let (txmsg, rxmsg) = mpsc::channel();
    let (txctrl, rxctrl) = mpsc::channel();
    thread::spawn(move || match MidiRx::new(txmsg /* , rxctrl*/) {
        Some(mut rx) => loop {
            if rx.periodic(rxctrl.try_recv()) == true {
                break;
            }
        },
        None => {
            println!("MIDI Rx thread does't work")
        }
    });
    (rxmsg, txctrl)
}
impl ElapseStack {
    pub fn new(ui_hndr: mpsc::Sender<String>) -> Option<Self> {
        match MidiTx::connect() {
            Ok(c) => {
                let mut part_vec = Vec::new();
                let mut elapse_vec = Vec::new();

                // Keyboard Part
                for i in 0..MAX_KBD_PART {
                    // 同じ Part を part_vec, elapse_vec 両方に繋げる
                    let pt = Part::new(i as u32, None);
                    part_vec.push(Rc::clone(&pt));
                    elapse_vec.push(pt as Rc<RefCell<dyn Elapse>>);
                }
                // Flow Part
                let flow = Flow::new(0, FLOW_PART as u32, false);
                elapse_vec.push(flow.clone() as Rc<RefCell<dyn Elapse>>);
                let pt = Part::new(FLOW_PART as u32, Some(flow));
                part_vec.push(Rc::clone(&pt));
                elapse_vec.push(pt as Rc<RefCell<dyn Elapse>>);
                // Damper Part
                let damper_part = DamperPart::new(DAMPER_PEDAL_PART as u32);
                elapse_vec.push(Rc::clone(&damper_part) as Rc<RefCell<dyn Elapse>>);

                let (rx_hndr, tx_ctrl) = gen_midirx_thread();
                Some(Self {
                    ui_hndr,
                    rx_hndr,
                    tx_ctrl,
                    mdx: c,
                    crnt_time: Instant::now(),
                    bpm_stock: DEFAULT_BPM,
                    beat_stock: Beat(4, 4),
                    during_play: false,
                    display_time: Instant::now(),
                    tg: TickGen::new(0),
                    part_vec: part_vec.clone(),
                    _damper_part: damper_part,
                    elapse_vec,
                    key_map: [0; (MAX_NOTE_NUMBER - MIN_NOTE_NUMBER + 1) as usize],
                    limit_for_deb: 0,
                })
            }
            Err(e) => {
                println!("{}", e);
                None
            }
        }
    }
    pub fn add_elapse(&mut self, elps: Rc<RefCell<dyn Elapse>>) {
        self.elapse_vec.push(elps);
    }
    pub fn _del_elapse(&mut self, search_id: ElapseId) {
        // 呼ぶとエラーが出る
        if let Some(remove_index) = self
            .elapse_vec
            .iter()
            .position(|x| x.borrow().id() == search_id)
        {
            self.elapse_vec.remove(remove_index);
        }
    }
    pub fn _get_part(&mut self, id: ElapseId) -> Option<Rc<RefCell<Part>>> {
        if let Some(index) = self.part_vec.iter().position(|x| x.borrow().id() == id) {
            let part = Rc::clone(&self.part_vec[index]);
            Some(part)
        } else {
            None
        }
    }
    pub fn get_phr(&self, part_num: usize) -> Option<Rc<RefCell<PhraseLoop>>> {
        self.part_vec[part_num].borrow().get_phr()
    }
    pub fn get_cmps(&self, part_num: usize) -> Option<Rc<RefCell<CompositionLoop>>> {
        self.part_vec[part_num].borrow().get_cmps()
    }
    pub fn get_flow(&self) -> Option<Rc<RefCell<Flow>>> {
        self.part_vec[FLOW_PART].borrow().get_flow()
    }
    pub fn tg(&self) -> &TickGen {
        &self.tg
    }
    pub fn inc_key_map(&mut self, key_num: u8, vel: u8, pt: u8) {
        self.key_map[(key_num - MIN_NOTE_NUMBER) as usize] += 1;
        let key_disp = format!("9{}/{}/{}", key_num, vel, pt);
        self.send_msg_to_ui(&key_disp);
    }
    pub fn dec_key_map(&mut self, key_num: u8) -> SameKeyState {
        let idx = (key_num - MIN_NOTE_NUMBER) as usize;
        if self.key_map[idx] > 1 {
            self.key_map[idx] -= 1;
            SameKeyState::MORE
        } else if self.key_map[idx] == 1 {
            self.key_map[idx] = 0;
            SameKeyState::LAST
        } else {
            SameKeyState::NOTHING
        }
    }
    pub fn set_phrase_vari(&self, part_num: usize, vari_num: usize) {
        self.part_vec[part_num]
            .borrow_mut()
            .set_phrase_vari(vari_num);
    }
    pub fn set_loop_end(&self, part_num: usize) {
        self.part_vec[part_num].borrow_mut().set_loop_end();
    }
    pub fn midi_out(&mut self, status: u8, data1: u8, data2: u8) {
        self.mdx.midi_out(status, data1, data2, true);
    }
    pub fn midi_out_flow(&mut self, status: u8, data1: u8, data2: u8) {
        self.mdx.midi_out(status, data1, data2, false);
    }
    pub fn midi_out_ext(&mut self, status: u8, data1: u8, data2: u8) {
        self.mdx.midi_out_only_for_another(status, data1, data2);
    }
    //*******************************************************************
    //      Periodic
    //*******************************************************************
    pub fn periodic(&mut self, msg: Result<ElpsMsg, TryRecvError>) -> bool {
        self.crnt_time = Instant::now();

        // message 受信処理
        if self.handle_msg(msg) {
            self.send_msg_to_rx(ElpsMsg::Ctrl(MSG_CTRL_QUIT));
            return true;
        }

        //  for GUI
        self.update_gui();

        //  新tick計算
        let mut msrtop = false;
        let mut crnt_ = CrntMsrTick::default();
        if self.during_play {
            msrtop = self.tg.gen_tick(self.crnt_time);
            crnt_ = self.tg.get_crnt_msr_tick();
        };

        // 小節先頭ならば、beat/bpm のイベント調査
        if msrtop {
            println!(
                "<New measure! in stack_elapse> Msr: {} Max Debcnt: {}/{}",
                crnt_.msr,
                self.limit_for_deb,
                self.elapse_vec.len()
            );
            #[cfg(feature = "verbose")]
            println!("  All Elapse Obj. Num: {:?}", self.elapse_vec.len());
            self.limit_for_deb = 0;
            // change beat event
            if self.beat_stock != self.tg.get_beat() {
                let tick_for_onemsr =
                    (DEFAULT_TICK_FOR_ONE_MEASURE / self.beat_stock.1) * self.beat_stock.0;
                self.tg.change_beat_event(tick_for_onemsr, self.beat_stock);
            }
            // for GUI(8indicator)
            self.update_gui_at_msrtop();
        }

        //　MIDI Rx処理
        self.check_rcv_midi(&crnt_);

        if self.during_play {
            let mut debcnt = 0;
            loop {
                // 現measure/tick より前のイベントを持つ obj を返す
                if let Some(felps) = self.pick_up_first(&crnt_) {
                    #[cfg(feature = "verbose")]
                    {
                        let et = felps.borrow().id();
                        let mt = felps.borrow().next();
                        println!(
                            "@@@<{:>04}> pid: {:?}, sid: {:?}, type: {:?}, nmsr: {:?}, ntick: {:?}",
                            crnt_.tick, et.pid, et.sid, et.elps_type, mt.0, mt.1
                        );
                    }
                    felps.borrow_mut().process(&crnt_, self);
                    debcnt += 1;
                    assert!(debcnt < 100, "Last Tick:{:?}", crnt_.tick);
                } else {
                    break;
                }
            }
            if self.limit_for_deb < debcnt {
                self.limit_for_deb = debcnt;
            }

            // remove ended obj
            self.destroy_finished_elps();
        }

        // play 中でなければ return
        return false;
    }
    //*******************************************************************
    //      handle message
    //*******************************************************************
    fn handle_msg(&mut self, msg: Result<ElpsMsg, TryRecvError>) -> bool {
        match msg {
            Ok(n) => {
                match n {
                    Ctrl(m) => {
                        if m == MSG_CTRL_QUIT {
                            return true;
                        } else {
                            self.parse_elps_msg(n)
                        }
                    }
                    _ => self.parse_elps_msg(n),
                }
                //if n[0] == MSG_QUIT {return true;}
                //else {self.parse_msg(n);}
            }
            Err(TryRecvError::Disconnected) => return true, // Wrong!
            Err(TryRecvError::Empty) => return false,       // No event
        }
        return false;
    }
    fn parse_elps_msg(&mut self, msg: ElpsMsg) {
        match msg {
            Ctrl(m) => self.ctrl_msg(m),
            Sync(m) => self.sync(m),
            Rit(m) => self.rit(m),
            Set(m) => self.setting_cmnd(m),
            SetBeat(m) => self.set_beat(m),
            Phr(m0, m1, mv) => self.phrase(m0, m1, mv),
            Cmp(m0, mv) => self.composition(m0, mv),
            Ana(m0, m1, mv) => self.ana(m0, m1, mv),
            PhrX(m0, m1) => self.del_phrase(m0, m1),
            CmpX(m) => self.del_composition(m),
            AnaX(m0, m1) => self.del_ana(m0, m1),
            _ => (),
        }
    }
    fn ctrl_msg(&mut self, msg: i16) {
        if msg == MSG_CTRL_START {
            self.start(false);
        } else if msg == MSG_CTRL_STOP {
            self.stop();
        } else if msg == MSG_CTRL_PANIC {
            self.panic();
        } else if msg == MSG_CTRL_RESUME {
            self.start(true);
        }
    }
    fn send_msg_to_ui(&self, msg: &str) {
        match self.ui_hndr.send(msg.to_string()) {
            Err(e) => println!("Something happened on MPSC for UI! {}", e),
            _ => {}
        }
    }
    fn send_msg_to_rx(&self, msg: ElpsMsg) {
        match self.tx_ctrl.send(msg) {
            Err(e) => println!("Something happened on MPSC To MIDIRx! {}", e),
            _ => {}
        }
    }
    fn check_rcv_midi(&mut self, crnt_: &CrntMsrTick) {
        match self.rx_hndr.try_recv() {
            Ok(rxmsg) => match rxmsg {
                MIDIRx(sts, nt, vel, extra) => {
                    self.rcv_midi_msg(crnt_, sts, nt, vel, extra);
                }
                _ => (),
            },
            Err(TryRecvError::Disconnected) => {} // Wrong!
            Err(TryRecvError::Empty) => {}
        }
    }
    fn rcv_midi_msg(&mut self, crnt_: &CrntMsrTick, sts: u8, nt: u8, vel: u8, ex: u8) {
        if sts & 0x0f == 0x0a {
            // 0a ch <from another loopian>
            if !self.during_play {
                // pattern 再生中は、External Loopian とは繋がない
                if sts & 0xe0 == 0x80 {
                    // LED を光らせる
                    self.mdx.midi_out_for_led(sts, nt, vel);
                } else if sts & 0xf0 == 0xa0 {
                    // Flow Part に和音を設定する
                    if let Some(fl) = self.part_vec[FLOW_PART].borrow_mut().get_flow() {
                        fl.borrow_mut().set_chord_for_noplay(nt, vel, ex);
                    }
                }
            }
        } else {
            // 0b/0c ch <from ORBIT>
            if (sts & 0xe0) == 0x80 {
                // 再生中 & Note Message
                let pt = self.part_vec[FLOW_PART].clone();
                pt.borrow_mut()
                    .rcv_midi_in(self, crnt_, sts & 0xf0, nt, vel);
            } else if (sts & 0xf0) == 0xc0 {
                // PCN は Pattern 切り替えに使用する
                let key_disp = format!("@ptn{}", nt);
                self.send_msg_to_ui(&key_disp);
            }
        }
    }
    //*******************************************************************
    //      Control Message
    //*******************************************************************
    fn start(&mut self, resume: bool) {
        if self.during_play && !resume {
            return;
        }
        self.during_play = true;
        self.tg.start(self.crnt_time, self.bpm_stock, resume);
        for elps in self.elapse_vec.iter() {
            elps.borrow_mut().start();
        }
        self.send_msg_to_rx(ElpsMsg::Ctrl(MSG_CTRL_START));
        println!("<Start Playing! in stack_elapse>",);
    }
    fn panic(&mut self) {
        self.midi_out(0xb0, 0x78, 0x00);
    }
    fn stop(&mut self) {
        if !self.during_play {
            return;
        }
        self.during_play = false;
        let stop_vec = self.elapse_vec.to_vec();
        for elps in stop_vec.iter() {
            elps.borrow_mut().stop(self);
        }
        // destroy flag の立った elapse obj.を回収
        self.destroy_finished_elps();
    }
    //fn fermata(&mut self, _msg: Vec<i16>) {self.fermata_stock = true;}
    fn sync(&mut self, part: i16) {
        let mut sync_part = [false; MAX_KBD_PART];
        if part < MAX_KBD_PART as i16 {
            sync_part[part as usize] = true;
        } else if part == MSG_SYNC_LFT {
            sync_part[LEFT1] = true;
            sync_part[LEFT2] = true;
        } else if part == MSG_SYNC_RGT {
            sync_part[RIGHT1] = true;
            sync_part[RIGHT2] = true;
        } else if part == MSG_SYNC_ALL {
            for pt in sync_part.iter_mut() {
                *pt = true;
            }
        }
        for (i, pt) in sync_part.iter().enumerate() {
            if *pt {
                self.part_vec[i].borrow_mut().set_sync();
            }
        }
    }
    fn rit(&mut self, msg: [i16; 2]) {
        let strength_set: [(i16, i32); 3] =
            [(MSG_RIT_POCO, 98), (MSG_RIT_NRM, 90), (MSG_RIT_MLT, 80)];
        let strength_msg = msg[0] % 10;
        let bar = (msg[0] / 10) as i32;
        let target_bpm: i16;
        let strength = strength_set
            .into_iter()
            .find(|x| x.0 == strength_msg)
            .unwrap_or(strength_set[0]);
        if msg[1] == MSG2_RIT_ATMP {
            target_bpm = self.tg.get_bpm();
        } else if msg[1] == MSG2_RIT_FERMATA {
            target_bpm = 0;
        } else {
            target_bpm = msg[1];
        }
        self.tg
            .start_rit(self.crnt_time, strength.1, bar, target_bpm);
    }
    fn setting_cmnd(&mut self, msg: [i16; 2]) {
        if msg[0] == MSG_SET_BPM {
            self.bpm_stock = msg[1];
            self.tg.change_bpm(msg[1])
        } else if msg[0] == MSG_SET_KEY {
            self.part_vec
                .iter()
                .for_each(|x| x.borrow_mut().change_key(msg[1] as u8));
        } else if msg[0] == MSG_SET_TURN {
            self.part_vec
                .iter_mut()
                .for_each(|x| x.borrow_mut().set_turnnote(msg[1]));
        } else if msg[0] == MSG_SET_CRNT_MSR {
            self.tg.set_crnt_msr(msg[1] as i32);
        }
    }
    fn set_beat(&mut self, msg: [i16; 2]) {
        self.beat_stock = Beat(msg[0] as i32, msg[1] as i32);
        self.sync(MSG_SYNC_ALL);
    }
    fn phrase(&mut self, part_num: i16, vari_num: i16, evts: PhrData) {
        println!(
            "Received Phrase Message! Part: {}, variation: {}",
            part_num, vari_num
        );
        self.part_vec[part_num as usize]
            .borrow_mut()
            .rcv_phr_msg(evts, vari_num as usize);
    }
    fn composition(&mut self, part_num: i16, evts: ChordData) {
        println!("Received Composition Message! Part: {}", part_num);
        self.part_vec[part_num as usize]
            .borrow_mut()
            .rcv_cmps_msg(evts);
    }
    fn ana(&mut self, part_num: i16, vari_num: i16, evts: AnaData) {
        println!(
            "Received Analysis Message! Part: {}, variation: {}",
            part_num, vari_num
        );
        self.part_vec[part_num as usize]
            .borrow_mut()
            .rcv_ana_msg(evts, vari_num as usize);
    }
    fn del_phrase(&mut self, part_num: i16, vari_num: i16) {
        self.part_vec[part_num as usize]
            .borrow_mut()
            .rcv_phr_msg(PhrData::empty(), vari_num as usize);
        self.part_vec[part_num as usize]
            .borrow_mut()
            .rcv_ana_msg(AnaData::empty(), vari_num as usize);
    }
    fn del_composition(&mut self, part_num: i16) {
        self.part_vec[part_num as usize]
            .borrow_mut()
            .rcv_cmps_msg(ChordData::empty());
    }
    fn del_ana(&mut self, part_num: i16, vari_num: i16) {
        self.part_vec[part_num as usize]
            .borrow_mut()
            .rcv_ana_msg(AnaData::empty(), vari_num as usize);
    }
    //*******************************************************************
    //      Pick out playable
    //*******************************************************************
    fn pick_up_first(&self, crnt_: &CrntMsrTick) -> Option<Rc<RefCell<dyn Elapse>>> {
        let mut first: Option<Rc<RefCell<dyn Elapse>>> = None;
        for elps in self.elapse_vec.iter() {
            let (msr, tick) = elps.borrow().next();
            if (msr == crnt_.msr && tick <= crnt_.tick) || msr < crnt_.msr {
                // 現在のタイミングより前のイベントがあれば
                if let Some(felps) = first.clone() {
                    let (msrx, tickx) = felps.borrow().next();
                    if (msr < msrx)
                        || ((msr == msrx) && (tick < tickx))
                        || ((msr == msrx)
                            && (tick == tickx)
                            && (felps.borrow().prio() > elps.borrow().prio()))
                    {
                        first = Some(elps.clone());
                    }
                } else {
                    first = Some(elps.clone());
                }
            }
        }
        first
    }
    fn _pick_out_playable(&self, crnt_: &CrntMsrTick) -> Vec<Rc<RefCell<dyn Elapse>>> {
        let mut playable: Vec<Rc<RefCell<dyn Elapse>>> = Vec::new();
        for elps in self.elapse_vec.iter() {
            let (msr, tick) = elps.borrow().next();
            if (msr == crnt_.msr && tick <= crnt_.tick) || msr < crnt_.msr {
                // 現在のタイミングより前のイベントがあれば
                if playable.len() == 0 {
                    // playable にまだ何も無ければ、普通に push
                    playable.push(Rc::clone(&elps));
                } else {
                    // playable に、時間順になるように挿入
                    let mut after_break = false;
                    for (i, one_plabl) in playable.iter().enumerate() {
                        let (msrx, tickx) = one_plabl.borrow().next();
                        if (msr < msrx)
                            || ((msr == msrx)
                                && ((tick < tickx)
                                    || ((tick == tickx)
                                        && (one_plabl.borrow().prio() > elps.borrow().prio()))))
                        {
                            playable.insert(i, Rc::clone(&elps));
                            after_break = true;
                            break;
                        }
                    }
                    if !after_break {
                        // 条件にはまらなければ最後に入れる
                        playable.push(Rc::clone(&elps));
                    }
                }
            }
        }
        playable
    }
    fn destroy_finished_elps(&mut self) {
        loop {
            let mut removed_num: i32 = -1;
            for (i, elps) in self.elapse_vec.iter().enumerate() {
                if elps.borrow().destroy_me() {
                    self.elapse_vec.remove(i);
                    removed_num = i as i32;
                    break;
                }
            }
            if removed_num == -1 {
                break;
            }
        }
    }
    //*******************************************************************
    //      Update GUI
    //*******************************************************************
    fn update_gui_at_msrtop(&mut self) {
        // key
        let key_disp = "0_".to_string();
        self.send_msg_to_ui(&key_disp);
        // beat
        let beat = self.tg.get_beat();
        let beat_disp = format!("2{}/{}", beat.0, beat.1);
        self.send_msg_to_ui(&beat_disp);
    }
    fn update_gui(&mut self) {
        if self.crnt_time - self.display_time > Duration::from_millis(50) {
            self.display_time = self.crnt_time;
            // bpm
            let bpm_num = if self.during_play {
                self.tg.get_real_bpm()
            } else {
                self.bpm_stock
            };
            let bpm_disp = format!("1{}", bpm_num);
            self.send_msg_to_ui(&bpm_disp);
            // tick
            let (m, b, t, _c) = self.tg.get_tick();
            let play = if self.during_play { ">" } else { "" };
            let tick_disp = format!("3{} {} : {} : {:>03}", play.to_string(), m, b, t);
            self.send_msg_to_ui(&tick_disp);
            // part
            self.part_vec.iter().for_each(|x| {
                let crnt_ = self.tg.get_crnt_msr_tick();
                let part_ind = x.borrow().gen_part_indicator(&crnt_);
                self.send_msg_to_ui(&part_ind);
            });
        }
    }
}
