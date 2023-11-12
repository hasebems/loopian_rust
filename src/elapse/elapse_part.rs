//  Created by Hasebe Masahiko on 2023/01/31
//  Copyright (c) 2023 Hasebe Masahiko.
//  Released under the MIT license
//  https://opensource.org/licenses/mit-license.php
//
use std::rc::Rc;
use std::cell::RefCell;

use crate::elapse::elapse_flow::Flow;
use crate::lpnlib::*;
use super::elapse::*;
use super::elapse_loop::{Loop, PhraseLoop, CompositionLoop, DamperLoop};
use super::tickgen::CrntMsrTick;
use super::stack_elapse::ElapseStack;
use super::ug_content::UgContent;

#[derive(Debug,Copy,Clone)]
struct PartBasicPrm {
    part_num: u32,
    keynote: u8,
    sync_flag: bool,
}

//*******************************************************************
//          Phrase Loop Manager Struct
//*******************************************************************
struct PhrLoopManager {
    first_msr_num: i32,
    max_loop_msr: i32,
    whole_tick: i32,
    loop_cntr: u32,         // loop sid
    new_data_stock: Vec<UgContent>,
    whole_tick_stock: [i16;MAX_PHRASE],
    new_ana_stock: Vec<UgContent>,
    loop_phrase: Option<Rc<RefCell<PhraseLoop>>>,
    vari_reserve: usize,      // 0:no rsv, 1-9: rsv
    state_reserve: bool,
    turnnote: i16,
}
impl PhrLoopManager {
    pub fn new() -> Self {
        let mut no_data: Vec<UgContent> = Vec::new();
        for _ in 0..MAX_PHRASE {no_data.push(UgContent::new());}
        Self {
            first_msr_num: 0,
            max_loop_msr: 0,
            whole_tick: 0,
            loop_cntr: 0,
            new_data_stock: no_data.clone(),
            whole_tick_stock: [0;MAX_PHRASE],
            new_ana_stock: no_data,
            loop_phrase: None,
            vari_reserve: 0,
            state_reserve: false,
            turnnote: DEFAULT_TURNNOTE,
        }
    }
    pub fn start(&mut self) {
        self.first_msr_num = 0;
    }
    pub fn process(&mut self, crnt_: &CrntMsrTick, estk: &mut ElapseStack, pbp: PartBasicPrm) {
        if self.vari_reserve != 0 {
            if let Some(phr) = self.loop_phrase.as_mut() {
                phr.borrow_mut().set_destroy();
            }
            self.new_loop(crnt_.msr, crnt_.tick_for_onemsr, estk, pbp);
        }
        else if self.state_reserve {
            // 前小節にて phrase/pattern 指定された時
            if crnt_.msr == 0 {
                // 今回 start したとき
                self.state_reserve = false;
                self.new_loop(crnt_.msr, crnt_.tick_for_onemsr, estk, pbp);
            }
            else if self.max_loop_msr == 0 {
                // データのない状態で start し、今回初めて指定された時
                self.state_reserve = false;
                self.new_loop(crnt_.msr, crnt_.tick_for_onemsr, estk, pbp);
            }
            else if self.max_loop_msr != 0 &&
              (crnt_.msr - self.first_msr_num)%(self.max_loop_msr) == 0 {
                // 前小節にて Loop Obj が終了した時
                self.state_reserve = false;
                self.new_loop(crnt_.msr, crnt_.tick_for_onemsr, estk, pbp);
            }
            else if self.max_loop_msr != 0 && pbp.sync_flag {
                // sync コマンドによる強制リセット
                self.state_reserve = false;
                if let Some(phr) = self.loop_phrase.as_mut() {
                    phr.borrow_mut().set_destroy();
                }
                self.new_loop(crnt_.msr, crnt_.tick_for_onemsr, estk, pbp);
            }
            else {
                // 現在の Loop Obj が終了していない時
                // state_reserve は持ち越す
            }
        }
        else if self.max_loop_msr != 0 &&
          (crnt_.msr - self.first_msr_num)%(self.max_loop_msr) == 0 {
            // 同じ Loop.Obj を生成する
            self.new_loop(crnt_.msr, crnt_.tick_for_onemsr, estk, pbp);
        }
        else {
               // 通常の Loop 中
        }
    }
    pub fn rcv_msg(&mut self, msg: UgContent, whole_tick: i16, vari_num: usize) {
        if vari_num < MAX_PHRASE {
            if msg.len() == 0 && whole_tick == 0 {self.new_data_stock[vari_num] = UgContent::new();}
            else {self.new_data_stock[vari_num] = msg;}
            self.whole_tick_stock[vari_num] = whole_tick;
            if vari_num == 0 {self.state_reserve = true;}
        }
    }
    pub fn rcv_ana(&mut self, msg: UgContent, vari_num: usize) {
        if vari_num < MAX_PHRASE {
            if msg.len() == 0 {self.new_ana_stock[vari_num] = UgContent::new();}
            else {self.new_ana_stock[vari_num] = msg;}
            if vari_num == 0 {self.state_reserve = true;}
        }
    }
    pub fn get_phr(&self) -> Option<Rc<RefCell<PhraseLoop>>> {
        self.loop_phrase.clone()    // 重いclone()?
    }
    pub fn gen_msrcnt(&self, crnt_msr: i32) -> String {
        if let Some(phr) = &self.loop_phrase {
            let denomirator = self.max_loop_msr;
            let numerator = crnt_msr - phr.borrow().first_msr_num() + 1;    // 1origin
            format!("{}/{}", numerator, denomirator)
        }
        else {
            String::from("---")
        }
    }
    pub fn set_turnnote(&mut self, tn: i16) {
        self.turnnote = tn;
    }
    pub fn reserve_vari(&mut self, vari_num: usize) {
        self.vari_reserve = vari_num; // 1-9
    }
    fn new_loop(&mut self, msr: i32, tick_for_onemsr: i32, estk: &mut ElapseStack, pbp: PartBasicPrm) {
        let mut new_loop = false;
        self.first_msr_num = msr;

        // Phrase の更新
        let phrlen = self.new_data_stock[self.vari_reserve].len();
        let analen = self.new_ana_stock[self.vari_reserve].len();
        if phrlen != 0 && analen != 0 {
            self.gen_new_loop(msr, tick_for_onemsr, estk, pbp);
            new_loop = true;
        }
        self.vari_reserve = 0;

        if !new_loop {
            self.whole_tick = 0;
            self.loop_phrase = None;
        }
    }
    fn gen_new_loop(&mut self, msr: i32, tick_for_onemsr: i32, estk: &mut ElapseStack, pbp: PartBasicPrm) {
        let phr = &self.new_data_stock[self.vari_reserve];
        let ana = &self.new_ana_stock[self.vari_reserve];
        // 新しいデータが来ていれば、新たに Loop Obj.を生成
        self.whole_tick = self.whole_tick_stock[self.vari_reserve] as i32;
        if self.whole_tick == 0 {
            self.state_reserve = true; // 次小節冒頭で呼ばれるように
            self.loop_phrase = None;
            self.max_loop_msr = 0;
            return;
        }

        // その時の beat 情報で、whole_tick を loop_measure に換算
        let plus_one = if self.whole_tick%tick_for_onemsr == 0 {0} else {1};
        self.max_loop_msr = self.whole_tick/tick_for_onemsr + plus_one;

        self.loop_cntr += 1;
        let lp = PhraseLoop::new(
            self.loop_cntr, 
            pbp.part_num, 
            pbp.keynote,
            msr,
            phr.copy_to(),
            ana.copy_to(),
            self.whole_tick,
            self.turnnote);
        self.loop_phrase = Some(Rc::clone(&lp));
        estk.add_elapse(lp);
        println!("New Phrase Loop! --whole tick: {}", self.whole_tick);
    }
}

//*******************************************************************
//          Composition Loop Manager Struct
//*******************************************************************
struct CmpsLoopManager {
    first_msr_num: i32,
    max_loop_msr: i32,
    whole_tick: i32,
    loop_cntr: u32,         // loop sid
    new_data_stock: UgContent,
    whole_tick_stock: i16,
    loop_cmps: Option<Rc<RefCell<CompositionLoop>>>,
    state_reserve: bool,
}
impl CmpsLoopManager {
    pub fn new() -> Self {
        Self {
            first_msr_num: 0,
            max_loop_msr: 0,
            whole_tick: 0,
            loop_cntr: 0,
            new_data_stock: UgContent::new(),
            whole_tick_stock: 0,
            loop_cmps: None,
            state_reserve: false,
        }
    }
    pub fn start(&mut self) {
        self.first_msr_num = 0;
    }
    pub fn process(&mut self, crnt_: &CrntMsrTick, estk: &mut ElapseStack, pbp: PartBasicPrm) {
        if self.state_reserve {
            // 前小節にて phrase/pattern 指定された時
            if crnt_.msr == 0 {
                // 今回 start したとき
                self.state_reserve = false;
                self.new_loop(crnt_, estk, pbp);
            }
            else if self.max_loop_msr == 0 {
                // データのない状態で start し、今回初めて指定された時
                self.state_reserve = false;
                self.new_loop(crnt_, estk, pbp);
            }
            else if self.max_loop_msr != 0 &&
              (crnt_.msr - self.first_msr_num)%(self.max_loop_msr) == 0 {
                // 前小節にて Loop Obj が終了した時
                self.state_reserve = false;
                self.new_loop(crnt_, estk, pbp);
            }
            else if self.max_loop_msr != 0 && pbp.sync_flag {
                // sync コマンドによる強制リセット
                self.state_reserve = false;
                if let Some(phr) = self.loop_cmps.as_mut() {
                    phr.borrow_mut().set_destroy();
                }
                self.new_loop(crnt_, estk, pbp);
            }
            else {
                // 現在の Loop Obj が終了していない時
                // state_reserve は持ち越す
            }
        }
        else if self.max_loop_msr != 0 &&
          (crnt_.msr - self.first_msr_num)%(self.max_loop_msr) == 0 {
            // 同じ Loop.Obj を生成する
            self.new_loop(crnt_, estk, pbp);
        }
    }
    pub fn rcv_msg(&mut self, msg: UgContent, whole_tick: i16) {
        //println!("Composition Msg: {:?}", msg);
        if msg.len() == 0 && whole_tick == 0 {self.new_data_stock = UgContent::new();}
        else {self.new_data_stock = msg;}
        self.state_reserve = true;
        self.whole_tick_stock = whole_tick;
    }
    pub fn get_cmps(&self) -> Option<Rc<RefCell<CompositionLoop>>> {
        self.loop_cmps.clone()    // 重いclone()?
    }
    pub fn gen_chord_name(&self) -> String {
        if let Some(cmps) = &self.loop_cmps {
            cmps.borrow().get_chord_name()
        }
        else {String::from("")}
    }
    fn new_loop(&mut self, crnt_: &CrntMsrTick, estk: &mut ElapseStack, pbp: PartBasicPrm) {
        // 新たに Loop Obj.を生成
        if self.new_data_stock.len() != 0 {
            println!("New Composition Loop!");
            self.first_msr_num = crnt_.msr;    // 計測開始の更新
            self.whole_tick = self.whole_tick_stock as i32;

            // その時の beat 情報で、whole_tick を loop_measure に換算
            let plus_one = if self.whole_tick%crnt_.tick_for_onemsr == 0 {0} else {1};
            self.max_loop_msr = self.whole_tick/crnt_.tick_for_onemsr + plus_one;

            if self.whole_tick == 0 {
                self.state_reserve = true; // 次小節冒頭で呼ばれるように
                self.loop_cmps = None;
                return;
            }

            self.loop_cntr += 1;
            let cmplp = CompositionLoop::new(
                self.loop_cntr,
                pbp.part_num, 
                pbp.keynote,
                crnt_.msr,
                self.new_data_stock.copy_to(),
                self.whole_tick);
            cmplp.borrow_mut().process(crnt_,estk); // 起動後、初回のみ process を呼び、同タイミングの再生を行う
            self.loop_cmps = Some(Rc::clone(&cmplp));
            estk.add_elapse(cmplp);
        }
        else {
            // 新しい Composition が空のとき
            self.max_loop_msr = 0;
            self.whole_tick = 0;
            self.loop_cntr = 0;
            self.state_reserve = true;
            self.loop_cmps = None;
        }
    }
}
//*******************************************************************
//          Damper Loop Manager Struct
//*******************************************************************
struct DamperLoopManager {
    first_msr_num: i32,
    loop_dmpr: Option<Rc<RefCell<DamperLoop>>>,
    loop_cntr: u32,
}
impl DamperLoopManager {
    pub fn new() -> Self {
        Self {
            first_msr_num: 0,
            loop_dmpr: None,
            loop_cntr: 0,
        }
    }
    pub fn start(&mut self) {self.first_msr_num = 0;}
    pub fn process(&mut self, crnt_: &CrntMsrTick, estk: &mut ElapseStack, pbp: PartBasicPrm) {
        let dp = DamperLoop::new(self.loop_cntr, pbp.part_num, crnt_.msr);
        self.loop_dmpr = Some(Rc::clone(&dp));
        estk.add_elapse(dp);
        self.loop_cntr += 1;
    }
}
//*******************************************************************
//          Part Struct
//*******************************************************************
pub struct Part {
    id: ElapseId,
    priority: u32,

    during_play: bool,
    keynote: u8,
    next_msr: i32,
    next_tick: i32,
    pm: PhrLoopManager,
    cm: CmpsLoopManager,
    dm: Option<DamperLoopManager>,
    flow: Option<Rc<RefCell<Flow>>>,
    sync_next_msr_flag: bool,
    start_flag: bool,
}
impl Part {
    pub fn new(num: u32) -> Rc<RefCell<Part>> {
        let new_id = ElapseId {pid:0, sid:num, elps_type: ElapseType::TpPart,};
        Rc::new(RefCell::new(Self {
            id: new_id,
            priority: PRI_PART,
            during_play: false,
            keynote: 0,
            next_msr: 0,
            next_tick: 0,
            pm: PhrLoopManager::new(),
            cm: CmpsLoopManager::new(),
            dm: if num as usize == DAMPER_PEDAL_PART {Some(DamperLoopManager::new())} else {None},
            flow: None,
            sync_next_msr_flag: false,
            start_flag: false,
        }))
    }
    pub fn change_key(&mut self, knt: u8) {
        self.keynote = knt;          // 0-11
        self.pm.state_reserve = true;
    }
    pub fn rcv_phr_msg(&mut self, msg: UgContent, whole_tick: i16, vari_num: usize) {
        self.pm.rcv_msg(msg, whole_tick, vari_num);
    }
    pub fn rcv_cmps_msg(&mut self, msg: UgContent, whole_tick: i16) {
        self.cm.rcv_msg(msg, whole_tick);
    }
    pub fn rcv_ana_msg(&mut self, msg_ana: UgContent, vari_num: usize) {
        self.pm.rcv_ana(msg_ana, vari_num);
    }
    pub fn get_phr(&self) -> Option<Rc<RefCell<PhraseLoop>>> {
        self.pm.get_phr()
    }
    pub fn get_cmps(&self) -> Option<Rc<RefCell<CompositionLoop>>> {
        self.cm.get_cmps()
    }
    pub fn get_flow(&self) -> Option<Rc<RefCell<Flow>>> {self.flow.clone()}
    pub fn set_turnnote(&mut self, tn: i16) {
        self.pm.set_turnnote(tn);
    }
    pub fn set_sync(&mut self) {
        self.pm.state_reserve = true;
        self.cm.state_reserve = true;
        self.sync_next_msr_flag = true;
    }
    pub fn gen_part_indicator(&self, crnt_: &CrntMsrTick) -> String {
        if self.pm.whole_tick != 0 {
            let msrcnt = self.pm.gen_msrcnt(crnt_.msr);
            let chord_name = self.cm.gen_chord_name();
            format!("{}{} {}",self.id.sid+4, msrcnt, chord_name)
        }
        else if self.flow.is_some() && self.during_play {
            let chord_name = self.cm.gen_chord_name();
            format!("{}Flow {}",self.id.sid+4, chord_name)
        }
        else {format!("{}---",self.id.sid+4)}
    }
    pub fn activate_flow(&mut self, estk: &mut ElapseStack) {
        if self.flow.is_none() {
            let fl = Flow::new(0, self.id.sid, self.during_play);
            self.flow = Some(Rc::clone(&fl));
            estk.add_elapse(fl);
        }
    }
    pub fn deactivate_flow(&mut self) {
        if let Some(fl) = &self.flow {
            fl.borrow_mut().deactivate();
            self.flow = None;
        }
    }
    pub fn rcv_midi_in(&mut self, crnt_: &CrntMsrTick, status:u8, locate:u8, vel:u8) {
        if let Some(fl) = &self.flow {fl.borrow_mut().rcv_midi(crnt_, status, locate, vel);}
    }
    pub fn set_phrase_vari(&mut self, vari_num: usize) {
        self.pm.reserve_vari(vari_num);
    }
}
impl Elapse for Part {
    fn id(&self) -> ElapseId {self.id}      // id を得る
    fn prio(&self) -> u32 {self.priority}   // priority を得る
    fn next(&self) -> (i32, i32) {          // 次に呼ばれる小節番号、Tick数を返す
        (self.next_msr, self.next_tick)
    }
    fn start(&mut self) {      // User による start/play 時にコールされる
        self.during_play = true;
        self.start_flag = true;
        self.next_msr = 0;
        self.next_tick = 0;
        self.cm.start();
        self.pm.start();
        if let Some(dmpr) = self.dm.as_mut() {
            dmpr.start();
        }
    }
    fn stop(&mut self, _estk: &mut ElapseStack) {// User による stop 時にコールされる
        self.during_play = false;
    }
    fn process(&mut self, crnt_: &CrntMsrTick, estk: &mut ElapseStack) {    // 再生 msr/tick に達したらコールされる
        let pbp = PartBasicPrm {
                                    part_num: self.id.sid,
                                    keynote: self.keynote,
                                    sync_flag: self.sync_next_msr_flag,
                                };
        if self.start_flag {
            // Start 直後
            self.cm.process(crnt_, estk, pbp);
            self.pm.process(crnt_, estk, pbp);
            if let Some(dmpr) = self.dm.as_mut() {
                dmpr.process(crnt_, estk, pbp);
            }
            self.start_flag = false;
        }
        else if self.next_tick != 0 {
            // 小節最後のみ
            let cm_crnt = CrntMsrTick {msr:crnt_.msr+1, tick:0, tick_for_onemsr:crnt_.tick_for_onemsr};
            self.cm.process(&cm_crnt, estk, pbp);
        }
        else {
            // 小節先頭のみ
            self.pm.process(crnt_, estk, pbp);
            if let Some(dmpr) = self.dm.as_mut() {
                dmpr.process(crnt_, estk, pbp);
            }
            self.sync_next_msr_flag = false;
        }

        // 次回 process を呼ぶタイミング
        if self.next_tick == 0 {    // 小節最後の tick
            self.next_tick = crnt_.tick_for_onemsr - 1;
        }
        else {                      // 小節最初の tick
            self.next_msr = crnt_.msr + 1;
            self.next_tick = 0;
        }
    }
    fn rcv_sp(&mut self, _msg: ElapseMsg, _msg_data: u8) {}
    fn destroy_me(&self) -> bool {   // 自クラスが役割を終えた時に True を返す
        false
    }
}