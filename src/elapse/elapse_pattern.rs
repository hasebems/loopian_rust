//  Created by Hasebe Masahiko on 2024/10/12
//  Copyright (c) 2024 Hasebe Masahiko.
//  Released under the MIT license
//  https://opensource.org/licenses/mit-license.php
//
use std::cell::RefCell;
use std::rc::Rc;

use super::elapse::*;
use super::elapse_note::Note;
use super::note_translation::*;
use super::stack_elapse::ElapseStack;
use super::tickgen::CrntMsrTick;
use crate::cmd::txt2seq_cmps;
use crate::lpnlib::*;

//*******************************************************************
//          Dynamic Pattern Struct
//*******************************************************************
pub struct DynamicPattern {
    id: ElapseId,
    priority: u32,

    arp_available: bool,
    ptn_tick: i32,
    ptn_min_nt: i32,
    ptn_vel: i32,
    ptn_each_dur: i32,
    ptn_max_vce: i32,
    analys: Vec<AnaEvt>,

    part: u32,
    keynote: u8,
    play_counter: usize,
    last_note: i16,
    noped: bool,
    para_root_base: i16,
    staccato_rate: i32,

    // for super's member
    whole_tick: i32,
    destroy: bool,
    first_msr_num: i32,
    next_msr: i32,  //   次に呼ばれる小節番号が保持される
    next_tick: i32, //   次に呼ばれるTick数が保持される
}
impl DynamicPattern {
    pub fn new(
        sid: u32,
        pid: u32,
        part: u32, // loop pid
        keynote: u8,
        msr: i32, // crnt_msr
        ptn: PhrEvt,
        ana: Vec<AnaEvt>,
    ) -> Rc<RefCell<Self>> {
        // generate pedal
        let noped = ana
            .clone()
            .iter()
            .any(|x| x.mtype == TYPE_EXP && x.atype == NOPED);
        // generate para_note_base
        let mut para_root_base = 0;
        ana.iter().for_each(|x| {
            if x.mtype == TYPE_EXP && x.atype == PARA_ROOT {
                para_root_base = x.note;
            }
        });
        // generate staccato rate
        let mut staccato_rate = 100;
        ana.iter().for_each(|x| {
            if x.mtype == TYPE_EXP && x.atype == ARTIC {
                staccato_rate = x.cnt as i32;
            }
        });
        // new Dynamic Pattern
        Rc::new(RefCell::new(Self {
            id: ElapseId {
                pid,
                sid,
                elps_type: ElapseType::TpDynamicPattern,
            },
            arp_available: false,
            priority: PRI_DYNPTN,
            ptn_tick: ptn.tick as i32,
            ptn_min_nt: ptn.note as i32,
            ptn_vel: ptn.vel as i32,
            ptn_each_dur: ptn.each_dur as i32,
            ptn_max_vce: ptn.trns as i32,
            analys: ana,
            part,
            keynote,
            play_counter: 0,
            last_note: NO_NOTE as i16,
            noped,
            para_root_base,
            staccato_rate,

            // for super's member
            whole_tick: ptn.dur as i32,
            destroy: false,
            first_msr_num: msr,
            next_msr: msr,
            next_tick: 0,
        }))
    }
    pub fn get_noped(&self) -> bool {
        self.noped
    }
    fn generate_event(&mut self, crnt_: &CrntMsrTick, estk: &mut ElapseStack) -> i32 {
        if self.arp_available {
            // Arpeggio
        } else {
            // Cluster
            self.play_cluster(estk);
        }
        self.play_counter += 1;

        // 次回 tick 算出と終了の確認
        let next_tick = self.next_tick + self.ptn_each_dur;
        if next_tick >= crnt_.tick_for_onemsr || next_tick >= self.whole_tick {
            END_OF_DATA
        } else {
            next_tick
        }
    }
    fn play_cluster(&mut self, estk: &mut ElapseStack) {
        if let Some(cmps) = estk.get_cmps(self.part as usize) {
            let (rt, ctbl) = cmps.borrow().get_chord();
            let root: i16 = ROOT2NTNUM[rt as usize];
            let (tbl, _take_upper) = txt2seq_cmps::get_table(ctbl as usize);
            for i in tbl {
                let note = *i + root + self.keynote as i16;
                self.gen_note_ev(estk, note);
            }
        }
    }
    fn gen_note_ev(&mut self, estk: &mut ElapseStack, note: i16) {
        let mut crnt_ev = PhrEvt::default();
        crnt_ev.dur = self.ptn_each_dur as i16;
        crnt_ev.note = note + DEFAULT_NOTE_NUMBER as i16;
        crnt_ev.vel = self.ptn_vel as i16;

        //  Generate Note Struct
        if self.staccato_rate != 100 {
            let old = crnt_ev.dur as i32;
            crnt_ev.dur = ((old * self.staccato_rate) / 100) as i16;
        }

        let nt: Rc<RefCell<dyn Elapse>> = Note::new(
            self.play_counter as u32, //  read pointer
            self.id.sid,              //  loop.sid -> note.pid
            estk,
            &crnt_ev,
            self.keynote,
            format!(" / Pt:{} Lp:{}", &self.part, &self.id.sid),
            self.first_msr_num,
            self.ptn_tick + self.ptn_each_dur * (self.play_counter as i32),
            self.part,
        );
        estk.add_elapse(Rc::clone(&nt));
    }
}

//*******************************************************************
//          Elapse IF for Dynamic Pattern
//*******************************************************************
impl Elapse for DynamicPattern {
    /// id を得る
    fn id(&self) -> ElapseId {
        self.id
    }
    /// priority を得る
    fn prio(&self) -> u32 {
        self.priority
    }
    /// 次に呼ばれる小節番号、Tick数を返す
    fn next(&self) -> (i32, i32) {
        (self.next_msr, self.next_tick)
    }
    fn start(&mut self, _msr: i32) {} // User による start/play 時にコールされる
    /// User による stop 時にコールされる
    fn stop(&mut self, _estk: &mut ElapseStack) {
        self.next_tick = 0;
        self.next_msr = FULL;
        self.destroy = true;
    }
    /// 再生データを消去
    fn clear(&mut self, _estk: &mut ElapseStack) {
        self.analys = Vec::new();
        self.play_counter = 0;
        self.last_note = NO_NOTE as i16;
        self.next_msr = 0;
        self.next_tick = 0;
    }
    fn rcv_sp(&mut self, _msg: ElapseMsg, _msg_data: u8) {}
    /// 自クラスが役割を終えた時に True を返す
    fn destroy_me(&self) -> bool {
        self.destroy
    }
    /// 再生 msr/tick に達したらコールされる
    fn process(&mut self, crnt_: &CrntMsrTick, estk: &mut ElapseStack) {
        if self.destroy {
            return;
        }

        if crnt_.msr > self.next_msr || crnt_.tick >= self.whole_tick + self.ptn_tick {
            self.next_msr = FULL;
            self.destroy = true;
        } else if crnt_.tick >= self.next_tick {
            let next_tick = self.generate_event(crnt_, estk);
            if next_tick == END_OF_DATA {
                self.next_msr = FULL;
                self.destroy = true;
            } else {
                self.next_tick = next_tick;
            }
        }
    }
}
