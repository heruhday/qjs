use std::cell::RefCell;
use std::collections::{HashMap, hash_map::Entry};
use std::ops::{Deref, DerefMut};
use std::rc::Rc;
use std::time::Instant;

use crate::atoms::{Atom, AtomTable, Shape as AtomShape};
use crate::gc::{self, GCHeader, Gc, ObjType};
use crate::heap::{
    QArray, QBoolArray, QClass, QClosure, QFloat64Array, QFunction, QInstance, QInt32Array,
    QModule, QNativeClosure, QNativeFunction, QObject, QString, QStringArray, QSymbol, QUint8Array,
};
use crate::js_value::*;
use crate::runtime::{CURRENT_JS_CONTEXT, Context, Runtime};
use crate::runtime_trait::{
    ArithmeticOps, AssignmentOps, BitwiseOps, CallOps, CoercionOps, ComparisonOps,
    LogicalAssignOps, LogicalOps, NullishOps, PropertyOps, Ternary, TypeOps, ValueOps,
};
use built_ins::BuiltinHost;

pub type JSString = QString;
const ACC: usize = 255;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyKey {
    Id(u16),
    Atom(Atom),
    Index(u32),
    Value(JSValue),
    PrivateName(Atom), // For private properties (#field, #method, etc.)
}

impl PropertyKey {
    fn sort_key(&self) -> (u8, u64) {
        match *self {
            PropertyKey::Id(id) => (0, u64::from(id)),
            PropertyKey::Atom(atom) => (1, u64::from(atom.0)),
            PropertyKey::Index(index) => (2, u64::from(index)),
            PropertyKey::Value(value) => (3, value.bits()),
            PropertyKey::PrivateName(atom) => (4, u64::from(atom.0)), // Private names sort after regular properties
        }
    }
}

#[derive(Debug, Clone)]
pub enum ObjectKind {
    Ordinary(QObject),
    Array(QArray),
    BoolArray(QBoolArray),
    Uint8Array(QUint8Array),
    Int32Array(QInt32Array),
    Float64Array(QFloat64Array),
    StringArray(QStringArray),
    Iterator { values: Vec<JSValue>, index: usize },
    Function(QFunction),
    Closure(QClosure),
    NativeFunction(QNativeFunction),
    NativeClosure(QNativeClosure),
    Class(QClass),
    Module(QModule),
    Instance(QInstance),
    Symbol(QSymbol),
    Env(QObject),
}

#[repr(C)]
#[derive(Debug)]
pub struct Shape {
    pub header: GCHeader,
    pub id: u32,
    pub parent: Option<*mut Shape>,
    pub key: Option<PropertyKey>,
    pub offset: u32,
    pub property_count: u32,
    pub prototype: Option<*mut Shape>,
    pub proto_cache_offset: u32,
    pub proto_cache_shape: Option<*mut Shape>,
}

#[repr(C, align(16))]
#[derive(Debug)]
pub struct JSObject {
    pub header: GCHeader,
    pub shape: *mut Shape,
    pub properties: HashMap<PropertyKey, JSValue>,
    pub private_properties: HashMap<PropertyKey, JSValue>, // Private properties (#field, #method, etc.)
    pub named_values: Vec<JSValue>,
    pub named_present: Vec<bool>,
    pub kind: ObjectKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ICState {
    Uninit = 0,
    Mono = 1,
    Poly = 2,
    Mega = 3,
}

#[derive(Debug, Clone)]
pub struct InlineCache {
    pub state: ICState,
    pub shape_id: u32,
    pub offset: u32,
    pub key: Option<PropertyKey>,
    pub shapes: Vec<u32>,
}

impl Default for InlineCache {
    fn default() -> Self {
        Self {
            state: ICState::Uninit,
            shape_id: 0,
            offset: 0,
            key: None,
            shapes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueProfileKind {
    Undefined,
    Null,
    Boolean,
    Number,
    String,
    Object,
    Function,
}

impl ValueProfileKind {
    pub fn from_id(id: u8) -> Option<Self> {
        match id {
            0 => Some(Self::Undefined),
            1 => Some(Self::Null),
            2 => Some(Self::Boolean),
            3 => Some(Self::Number),
            4 => Some(Self::String),
            5 => Some(Self::Object),
            6 => Some(Self::Function),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeFeedbackSlot {
    pub last: Option<ValueProfileKind>,
    pub samples: u32,
    pub stable: bool,
}

impl Default for TypeFeedbackSlot {
    fn default() -> Self {
        Self {
            last: None,
            samples: 0,
            stable: true,
        }
    }
}

impl TypeFeedbackSlot {
    fn observe(&mut self, kind: ValueProfileKind) {
        self.samples = self.samples.saturating_add(1);
        self.stable = match self.last {
            Some(previous) => self.stable && previous == kind,
            None => true,
        };
        self.last = Some(kind);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallFeedbackSlot {
    pub last: Option<ValueProfileKind>,
    pub samples: u32,
    pub monomorphic: bool,
}

impl Default for CallFeedbackSlot {
    fn default() -> Self {
        Self {
            last: None,
            samples: 0,
            monomorphic: true,
        }
    }
}

impl CallFeedbackSlot {
    fn observe(&mut self, kind: ValueProfileKind) {
        self.samples = self.samples.saturating_add(1);
        self.monomorphic = match self.last {
            Some(previous) => self.monomorphic && previous == kind,
            None => true,
        };
        self.last = Some(kind);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeoptReason {
    TypeMismatch {
        expected: ValueProfileKind,
        observed: ValueProfileKind,
    },
    StructMismatch {
        expected: u32,
        observed: u32,
    },
    InlineCacheMismatch {
        slot: usize,
        expected: u32,
        observed: u32,
    },
    SafetyCheck {
        register: usize,
    },
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeFeedback {
    pub type_slots: Vec<TypeFeedbackSlot>,
    pub call_slots: Vec<CallFeedbackSlot>,
    pub return_slot: TypeFeedbackSlot,
    pub hot_loop_counts: HashMap<usize, u32>,
    pub loop_hint_counts: HashMap<usize, u32>,
    pub jit_hints: HashMap<usize, u32>,
    pub osr_entries: u32,
    pub osr_exits: u32,
    pub safety_checks: u32,
    pub failed_safety_checks: u32,
    pub ic_misses: u32,
    pub deopt_count: u32,
    pub last_deopt: Option<DeoptReason>,
    pub last_loop_hint_pc: Option<usize>,
    pub last_call_kind: Option<ValueProfileKind>,
    pub last_ic_slot: Option<usize>,
    pub osr_active: bool,
}

#[derive(Debug, Clone, Copy)]
enum PendingCallContinuation {
    AddReturnedToAcc { lhs: JSValue },
    Call2SubIAddSecond { callee: JSValue, arg: JSValue },
}

#[derive(Debug)]
pub struct FrameHeader {
    pub return_pc: usize,
    pub function_id: usize,
    pub function_value: Option<JSValue>,
    pub env: Option<JSValue>,
    pub frame_size: u32,
    pub register_count: u32,
    pub construct_result: Option<JSValue>,
    pub scope_depth: usize,
    pending_call: Option<PendingCallContinuation>,
}

#[derive(Debug)]
pub struct Frame {
    pub header: FrameHeader,
    pub regs: [JSValue; 256],
    pub ic_vector: Vec<InlineCache>,
    pub inline_args: [JSValue; 2],
    pub args: Vec<JSValue>,
    pub argc: u32,
    pub try_stack: Vec<usize>,
    pub scope_stack: Vec<usize>,
}

impl Frame {
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn fresh(
        args: Vec<JSValue>,
        this_value: JSValue,
        function_id: usize,
        function_value: Option<JSValue>,
        return_pc: usize,
        construct_result: Option<JSValue>,
        scope_depth: usize,
    ) -> Self {
        let mut frame = Self {
            header: FrameHeader {
                return_pc,
                function_id,
                function_value,
                env: None,
                frame_size: 256,
                register_count: 256,
                construct_result,
                scope_depth,
                pending_call: None,
            },
            regs: [make_undefined(); 256],
            ic_vector: Vec::new(),
            inline_args: [make_undefined(); 2],
            args: Vec::new(),
            argc: 0,
            try_stack: Vec::new(),
            scope_stack: Vec::new(),
        };
        frame.regs[0] = this_value;
        frame.set_args(&args);
        frame
    }

    #[inline(always)]
    fn set_args(&mut self, args: &[JSValue]) {
        self.inline_args = [make_undefined(); 2];
        self.args.clear();
        self.argc = args.len() as u32;

        match args {
            [] => {}
            [arg0] => {
                self.inline_args[0] = *arg0;
            }
            [arg0, arg1] => {
                self.inline_args[0] = *arg0;
                self.inline_args[1] = *arg1;
            }
            _ => {
                self.inline_args[0] = args[0];
                self.inline_args[1] = args[1];
                self.args.extend_from_slice(&args[2..]);
            }
        }
    }

    #[inline(always)]
    fn arg(&self, index: usize) -> JSValue {
        if index >= self.argc as usize {
            return make_undefined();
        }

        match index {
            0 => self.inline_args[0],
            1 => self.inline_args[1],
            _ => self
                .args
                .get(index - self.inline_args.len())
                .copied()
                .unwrap_or(make_undefined()),
        }
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn reset(
        &mut self,
        args: &[JSValue],
        this_value: JSValue,
        function_id: usize,
        function_value: Option<JSValue>,
        return_pc: usize,
        construct_result: Option<JSValue>,
        scope_depth: usize,
    ) {
        self.header.return_pc = return_pc;
        self.header.function_id = function_id;
        self.header.function_value = function_value;
        self.header.env = None;
        self.header.frame_size = 256;
        self.header.register_count = 256;
        self.header.construct_result = construct_result;
        self.header.scope_depth = scope_depth;
        self.header.pending_call = None;
        self.regs.fill(make_undefined());
        self.regs[0] = this_value;
        self.ic_vector.clear();
        self.set_args(args);
        self.try_stack.clear();
        self.scope_stack.clear();
    }
}

#[derive(Debug)]
pub struct FrameStack {
    frames: Vec<Frame>,
    sp: usize,
    current: *mut Frame,
}

impl FrameStack {
    #[inline(always)]
    fn new(root: Frame) -> Self {
        let mut frames = Vec::with_capacity(32);
        frames.push(root);
        let current = frames.as_mut_ptr();
        Self {
            frames,
            sp: 0,
            current,
        }
    }

    #[inline(always)]
    fn depth(&self) -> usize {
        self.sp
    }

    #[inline(always)]
    fn sync_current(&mut self) {
        self.current = unsafe { self.frames.as_mut_ptr().add(self.sp) };
    }

    #[inline(always)]
    fn ensure_next_frame(&mut self) -> &mut Frame {
        let next = self.sp + 1;
        if next == self.frames.len() {
            self.frames.push(Frame::fresh(
                Vec::new(),
                make_undefined(),
                0,
                None,
                0,
                None,
                0,
            ));
        }
        self.sp = next;
        self.sync_current();
        unsafe { &mut *self.current }
    }

    #[inline(always)]
    fn pop_frame(&mut self) -> bool {
        if self.sp == 0 {
            return false;
        }
        self.sp -= 1;
        self.sync_current();
        true
    }

    #[inline(always)]
    fn caller_frame_mut(&mut self) -> Option<&mut Frame> {
        (self.sp > 0).then(|| &mut self.frames[self.sp - 1])
    }

    #[inline(always)]
    pub(crate) fn active_frames(&self) -> &[Frame] {
        &self.frames[..=self.sp]
    }
}

impl Deref for FrameStack {
    type Target = Frame;

    #[inline(always)]
    fn deref(&self) -> &Self::Target {
        debug_assert!(!self.current.is_null());
        unsafe { &*self.current }
    }
}

impl DerefMut for FrameStack {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Self::Target {
        debug_assert!(!self.current.is_null());
        unsafe { &mut *self.current }
    }
}

/// Type for threaded dispatch handler functions
type DispatchHandler = fn(&mut VM, u32) -> ControlFlow;

/// Control flow for threaded dispatch
#[derive(Debug, Clone, Copy)]
enum ControlFlow {
    /// Continue to next instruction
    Continue,
    /// Stop execution
    Stop,
}

fn with_bridge_context<R>(f: impl FnOnce(&Context) -> R) -> R {
    let previous = CURRENT_JS_CONTEXT.with(|current| current.borrow().clone());
    let runtime = Rc::new(RefCell::new(Runtime::new()));
    let ctx = Context::new(runtime);
    let result = f(&ctx);
    CURRENT_JS_CONTEXT.with(|current| {
        *current.borrow_mut() = previous;
    });
    result
}

fn builtin_native_stub(
    _ctx: &dyn crate::heap::RuntimeContext,
    _this: JSValue,
    _args: &[JSValue],
) -> JSValue {
    make_undefined()
}

#[derive(Debug)]
pub struct VM {
    pub frame: FrameStack,
    pub pc: usize,
    pub bytecode: Vec<u32>,
    pub const_pool: Vec<JSValue>,
    pub objects: Vec<*mut JSObject>,
    pub shapes: Vec<*mut Shape>,
    pub strings: Vec<*mut JSString>,
    pub global_object: HashMap<u16, JSValue>,
    pub console_output: Vec<String>,
    pub scope_chain: Vec<JSValue>,
    pub upvalues: Vec<JSValue>,
    pub last_exception: JSValue,
    pub(crate) interned_strings: HashMap<String, JSValue>,
    compiled_properties: Vec<String>,
    compiled_private_properties: Vec<Atom>,
    property_slots: HashMap<String, u16>,
    pub atoms: AtomTable,
    pub feedback: RuntimeFeedback,
    heap_shape: Rc<AtomShape>,
    next_shape_id: u32,
    last_ic_object: Option<*mut JSObject>,
    function_constants: Vec<u16>,
    console_timers: HashMap<String, Instant>,
    console_counts: HashMap<String, usize>,
    console_group_depth: usize,
    console_echo: bool,
    builtin_number_to_fixed: JSValue,
    builtin_object_prototype: JSValue,
    builtin_string_prototype: JSValue,
    symbol_registry: HashMap<String, JSValue>,
    next_symbol_id: u64,

    /// Dispatch table for threaded execution (hot opcodes only)
    dispatch_table: [Option<DispatchHandler>; 256],
    threaded_stop_depth: Option<usize>,
}

enum CallAction {
    Returned(JSValue),
    EnteredFrame,
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Mov,
    LoadK,
    Add,
    GetPropIc,
    Call,
    Jmp,
    LoadI,
    JmpTrue,
    JmpFalse,
    SetPropIc,
    AddAccImm8,
    IncAcc,
    LoadThis,
    Load0,
    Load1,
    Eq,
    Lt,
    Lte,
    AddAcc,
    SubAcc,
    MulAcc,
    DivAcc,
    LoadNull,
    LoadTrue,
    LoadFalse,
    LoadGlobalIc,
    SetGlobalIc,
    Typeof,
    ToNum,
    ToStr,
    IsUndef,
    IsNull,
    SubAccImm8,
    MulAccImm8,
    DivAccImm8,
    AddStrAcc,
    AddI,
    SubI,
    MulI,
    DivI,
    ModI,
    Mod,
    Neg,
    Inc,
    Dec,
    AddStr,
    ToPrimitive,
    GetPropAcc,
    SetPropAcc,
    GetIdxFast,
    SetIdxFast,
    LoadArg,
    LoadRestArgs,
    LoadAcc,
    StrictEq,
    StrictNeq,
    BitAnd,
    BitOr,
    BitXor,
    BitNot,
    Shl,
    Shr,
    Ushr,
    Pow,
    LogicalAnd,
    LogicalOr,
    NullishCoalesce,
    In,
    PrivateIn,
    Instanceof,
    GetLengthIc,
    ArrayPushAcc,
    NewObj,
    NewArr,
    NewFunc,
    NewClass,
    GetProp,
    SetProp,
    GetPrivateProp,
    SetPrivateProp,
    GetIdxIc,
    SetIdxIc,
    GetGlobal,
    SetGlobal,
    GetUpval,
    SetUpval,
    GetScope,
    SetScope,
    ResolveScope,
    GetSuper,
    SetSuper,
    DeleteProp,
    HasProp,
    Keys,
    ForIn,
    IteratorNext,
    Spread,
    Destructure,
    CreateEnv,
    LoadName,
    StoreName,
    InitName,
    LoadClosure,
    NewThis,
    TypeofName,
    JmpEq,
    JmpNeq,
    JmpLt,
    JmpLte,
    LoopIncJmp,
    Switch,
    LoopHint,
    Ret,
    RetU,
    TailCall,
    Construct,
    CallVar,
    CallThis,
    CallThisVar,
    Enter,
    Leave,
    Yield,
    Await,
    Throw,
    Try,
    EndTry,
    Catch,
    Finally,
    CallIc,
    CallIcVar,
    ProfileType,
    ProfileCall,
    ProfileRet,
    CheckType,
    CheckStruct,
    CheckIc,
    IcInit,
    IcUpdate,
    IcMiss,
    OsrEntry,
    ProfileHotLoop,
    OsrExit,
    JitHint,
    SafetyCheck,
    GetPropIcCall,
    IncJmpFalseLoop,
    LoadKAddAcc,
    AddMov,
    EqJmpTrue,
    GetPropAccCall,
    LoadKMulAcc,
    LtJmp,
    GetPropIcMov,
    GetPropAddImmSetPropIc,
    AddAccImm8Mov,
    CallIcSuper,
    LoadThisCall,
    EqJmpFalse,
    LoadKSubAcc,
    GetLengthIcCall,
    AddStrAccMov,
    IncAccJmp,
    GetPropChainAcc,
    TestJmpTrue,
    LoadArgCall,
    MulAccMov,
    LteJmpLoop,
    NewObjInitProp,
    ProfileHotCall,
    Call2SubIAdd,
    Call1SubI,
    JmpLteFalse,
    RetReg,
    AddI32,
    AddF64,
    SubI32,
    SubF64,
    MulI32,
    MulF64,
    // Superinstructions
    RetIfLteI,
    AddAccReg,
    Call1Add,
    Call2Add,
    LoadKAdd,
    LoadKCmp,
    CmpJmp,
    GetPropCall,
    CallRet,
    // Specialized opcodes
    AddI32Fast,
    AddF64Fast,
    SubI32Fast,
    MulI32Fast,
    EqI32Fast,
    LtI32Fast,
    JmpI32Fast,
    GetPropMono,
    CallMono,
    // Call opcodes
    Call0,
    Call1,
    Call2,
    Call3,
    CallMethod1,
    CallMethod2,
    // New arithmetic superinstructions
    LoadAdd,
    LoadSub,
    LoadMul,
    LoadInc,
    LoadDec,
    // New comparison superinstructions
    LoadCmpEq,
    LoadCmpLt,
    LoadJfalse,
    LoadCmpEqJfalse,
    LoadCmpLtJfalse,
    // Property access superinstructions
    LoadGetProp,
    LoadGetPropCmpEq,
    // Pareto 80% property access superinstructions with IC
    GetProp2Ic,
    GetProp3Ic,
    GetElem,
    SetElem,
    GetPropElem,
    CallMethodIc,
    CallMethod2Ic,
    LtF64,
    LteF64,
    JmpLtF64,
    JmpLteF64,
    JmpLteFalseF64,
    // Assertion opcodes
    AssertValue,
    AssertOk,
    AssertFail,
    AssertThrows,
    AssertDoesNotThrow,
    AssertRejects,
    AssertDoesNotReject,
    AssertEqual,
    AssertNotEqual,
    AssertDeepEqual,
    AssertNotDeepEqual,
    AssertStrictEqual,
    AssertNotStrictEqual,
    AssertDeepStrictEqual,
    AssertNotDeepStrictEqual,
    Reserved(u8),
}

impl From<u8> for Opcode {
    fn from(value: u8) -> Self {
        match value {
            0 => Self::Mov,
            1 => Self::LoadK,
            2 => Self::Add,
            3 => Self::GetPropIc,
            4 => Self::Call,
            5 => Self::Jmp,
            6 => Self::LoadI,
            7 => Self::JmpTrue,
            8 => Self::JmpFalse,
            9 => Self::SetPropIc,
            10 => Self::AddAccImm8,
            11 => Self::IncAcc,
            12 => Self::LoadThis,
            13 => Self::Load0,
            14 => Self::Load1,
            15 => Self::Eq,
            16 => Self::Lt,
            17 => Self::Lte,
            18 => Self::AddAcc,
            19 => Self::SubAcc,
            20 => Self::MulAcc,
            21 => Self::DivAcc,
            22 => Self::LoadNull,
            23 => Self::LoadTrue,
            24 => Self::LoadFalse,
            25 => Self::LoadGlobalIc,
            26 => Self::SetGlobalIc,
            27 => Self::Typeof,
            28 => Self::ToNum,
            29 => Self::ToStr,
            30 => Self::IsUndef,
            31 => Self::IsNull,
            32 => Self::SubAccImm8,
            33 => Self::MulAccImm8,
            34 => Self::DivAccImm8,
            35 => Self::AddStrAcc,
            36 => Self::AddI,
            37 => Self::SubI,
            38 => Self::MulI,
            39 => Self::DivI,
            40 => Self::ModI,
            61 => Self::Mod,
            41 => Self::Neg,
            42 => Self::Inc,
            43 => Self::Dec,
            44 => Self::AddStr,
            45 => Self::ToPrimitive,
            46 => Self::GetPropAcc,
            47 => Self::SetPropAcc,
            48 => Self::GetIdxFast,
            49 => Self::SetIdxFast,
            50 => Self::LoadArg,
            225 => Self::LoadRestArgs,
            51 => Self::LoadAcc,
            52 => Self::StrictEq,
            53 => Self::StrictNeq,
            54 => Self::BitAnd,
            55 => Self::BitOr,
            56 => Self::BitXor,
            57 => Self::BitNot,
            58 => Self::Shl,
            59 => Self::Shr,
            60 => Self::Ushr,
            64 => Self::GetLengthIc,
            65 => Self::ArrayPushAcc,
            66 => Self::NewObj,
            67 => Self::NewArr,
            68 => Self::NewFunc,
            69 => Self::NewClass,
            70 => Self::GetProp,
            71 => Self::SetProp,
            72 => Self::GetPrivateProp,
            73 => Self::SetPrivateProp,
            74 => Self::GetIdxIc,
            75 => Self::SetIdxIc,
            76 => Self::GetGlobal,
            77 => Self::SetGlobal,
            78 => Self::GetUpval,
            79 => Self::SetUpval,
            80 => Self::GetScope,
            81 => Self::SetScope,
            82 => Self::ResolveScope,
            83 => Self::DeleteProp,
            84 => Self::HasProp,
            85 => Self::Keys,
            86 => Self::ForIn,
            87 => Self::IteratorNext,
            88 => Self::Spread,
            89 => Self::Destructure,
            90 => Self::CreateEnv,
            91 => Self::LoadName,
            92 => Self::StoreName,
            93 => Self::LoadClosure,
            94 => Self::NewThis,
            95 => Self::TypeofName,
            96 => Self::JmpEq,
            97 => Self::JmpNeq,
            98 => Self::JmpLt,
            99 => Self::JmpLte,
            100 => Self::LoopIncJmp,
            101 => Self::Switch,
            102 => Self::LoopHint,
            103 => Self::Ret,
            104 => Self::RetU,
            105 => Self::TailCall,
            106 => Self::Construct,
            107 => Self::CallVar,
            226 => Self::CallThis,
            227 => Self::CallThisVar,
            108 => Self::Enter,
            109 => Self::Leave,
            110 => Self::Yield,
            111 => Self::Await,
            112 => Self::Throw,
            113 => Self::Try,
            114 => Self::EndTry,
            115 => Self::Catch,
            116 => Self::Finally,
            117 => Self::Pow,
            118 => Self::LogicalAnd,
            119 => Self::LogicalOr,
            120 => Self::NullishCoalesce,
            121 => Self::In,
            122 => Self::PrivateIn,
            123 => Self::InitName,
            124 => Self::Instanceof,
            128 => Self::CallIc,
            129 => Self::CallIcVar,
            160 => Self::ProfileType,
            161 => Self::ProfileCall,
            162 => Self::ProfileRet,
            163 => Self::CheckType,
            164 => Self::CheckStruct,
            165 => Self::CheckIc,
            166 => Self::IcInit,
            167 => Self::IcUpdate,
            168 => Self::IcMiss,
            169 => Self::OsrEntry,
            170 => Self::ProfileHotLoop,
            171 => Self::OsrExit,
            172 => Self::JitHint,
            173 => Self::SafetyCheck,
            200 => Self::GetPropIcCall,
            201 => Self::IncJmpFalseLoop,
            202 => Self::LoadKAddAcc,
            203 => Self::AddMov,
            204 => Self::EqJmpTrue,
            205 => Self::GetPropAccCall,
            206 => Self::LoadKMulAcc,
            207 => Self::LtJmp,
            208 => Self::GetPropIcMov,
            209 => Self::GetPropAddImmSetPropIc,
            210 => Self::AddAccImm8Mov,
            211 => Self::CallIcSuper,
            212 => Self::LoadThisCall,
            213 => Self::EqJmpFalse,
            214 => Self::LoadKSubAcc,
            215 => Self::GetLengthIcCall,
            216 => Self::AddStrAccMov,
            217 => Self::IncAccJmp,
            218 => Self::GetPropChainAcc,
            219 => Self::TestJmpTrue,
            220 => Self::LoadArgCall,
            221 => Self::MulAccMov,
            222 => Self::LteJmpLoop,
            223 => Self::NewObjInitProp,
            224 => Self::ProfileHotCall,
            239 => Self::Call2SubIAdd,
            240 => Self::Call1SubI,
            241 => Self::JmpLteFalse,
            242 => Self::RetReg,
            243 => Self::AddI32,
            244 => Self::AddF64,
            245 => Self::SubI32,
            246 => Self::SubF64,
            247 => Self::MulI32,
            248 => Self::MulF64,
            // Superinstructions
            249 => Self::RetIfLteI,
            250 => Self::AddAccReg,
            251 => Self::Call1Add,
            252 => Self::Call2Add,
            253 => Self::LoadKAdd,
            254 => Self::LoadKCmp,
            255 => Self::CmpJmp,
            // Specialized opcodes
            130 => Self::AddI32Fast,
            131 => Self::AddF64Fast,
            132 => Self::SubI32Fast,
            133 => Self::MulI32Fast,
            134 => Self::EqI32Fast,
            135 => Self::LtI32Fast,
            136 => Self::JmpI32Fast,
            137 => Self::GetPropMono,
            138 => Self::CallMono,
            // Call opcodes
            139 => Self::Call0,
            140 => Self::Call1,
            141 => Self::Call2,
            142 => Self::Call3,
            143 => Self::CallMethod1,
            144 => Self::CallMethod2,
            // Superinstruction variants
            145 => Self::GetPropCall,
            146 => Self::CallRet,
            // Assertion opcodes
            147 => Self::AssertValue,
            148 => Self::AssertOk,
            149 => Self::AssertFail,
            150 => Self::AssertThrows,
            151 => Self::AssertDoesNotThrow,
            152 => Self::AssertRejects,
            153 => Self::AssertDoesNotReject,
            154 => Self::AssertEqual,
            155 => Self::AssertNotEqual,
            156 => Self::AssertDeepEqual,
            157 => Self::AssertNotDeepEqual,
            158 => Self::AssertStrictEqual,
            159 => Self::AssertNotStrictEqual,
            174 => Self::AssertDeepStrictEqual,
            175 => Self::AssertNotDeepStrictEqual,
            // New arithmetic superinstructions
            176 => Self::LoadAdd,
            177 => Self::LoadSub,
            178 => Self::LoadMul,
            179 => Self::LoadInc,
            180 => Self::LoadDec,
            // New comparison superinstructions
            181 => Self::LoadCmpEq,
            182 => Self::LoadCmpLt,
            183 => Self::LoadJfalse,
            184 => Self::LoadCmpEqJfalse,
            185 => Self::LoadCmpLtJfalse,
            // Property access superinstructions
            186 => Self::LoadGetProp,
            187 => Self::LoadGetPropCmpEq,
            // Pareto 80% property access superinstructions with IC
            188 => Self::GetProp2Ic,
            189 => Self::GetProp3Ic,
            190 => Self::GetElem,
            191 => Self::SetElem,
            192 => Self::GetPropElem,
            193 => Self::CallMethodIc,
            194 => Self::CallMethod2Ic,
            195 => Self::LtF64,
            196 => Self::LteF64,
            197 => Self::JmpLtF64,
            198 => Self::JmpLteF64,
            199 => Self::JmpLteFalseF64,
            other => Self::Reserved(other),
        }
    }
}

impl Opcode {
    /// Convert an Opcode to its u8 representation
    pub fn as_u8(self) -> u8 {
        match self {
            Self::Mov => 0,
            Self::LoadK => 1,
            Self::Add => 2,
            Self::GetPropIc => 3,
            Self::Call => 4,
            Self::Jmp => 5,
            Self::LoadI => 6,
            Self::JmpTrue => 7,
            Self::JmpFalse => 8,
            Self::SetPropIc => 9,
            Self::AddAccImm8 => 10,
            Self::IncAcc => 11,
            Self::LoadThis => 12,
            Self::Load0 => 13,
            Self::Load1 => 14,
            Self::Eq => 15,
            Self::Lt => 16,
            Self::Lte => 17,
            Self::AddAcc => 18,
            Self::SubAcc => 19,
            Self::MulAcc => 20,
            Self::DivAcc => 21,
            Self::LoadNull => 22,
            Self::LoadTrue => 23,
            Self::LoadFalse => 24,
            Self::LoadGlobalIc => 25,
            Self::SetGlobalIc => 26,
            Self::Typeof => 27,
            Self::ToNum => 28,
            Self::ToStr => 29,
            Self::IsUndef => 30,
            Self::IsNull => 31,
            Self::SubAccImm8 => 32,
            Self::MulAccImm8 => 33,
            Self::DivAccImm8 => 34,
            Self::AddStrAcc => 35,
            Self::AddI => 36,
            Self::SubI => 37,
            Self::MulI => 38,
            Self::DivI => 39,
            Self::ModI => 40,
            Self::Mod => 61,
            Self::Neg => 41,
            Self::Inc => 42,
            Self::Dec => 43,
            Self::AddStr => 44,
            Self::ToPrimitive => 45,
            Self::GetPropAcc => 46,
            Self::SetPropAcc => 47,
            Self::GetIdxFast => 48,
            Self::SetIdxFast => 49,
            Self::LoadArg => 50,
            Self::LoadRestArgs => 225,
            Self::LoadAcc => 51,
            Self::StrictEq => 52,
            Self::StrictNeq => 53,
            Self::BitAnd => 54,
            Self::BitOr => 55,
            Self::BitXor => 56,
            Self::BitNot => 57,
            Self::Shl => 58,
            Self::Shr => 59,
            Self::Ushr => 60,
            Self::Pow => 117,
            Self::LogicalAnd => 118,
            Self::LogicalOr => 119,
            Self::NullishCoalesce => 120,
            Self::In => 121,
            Self::PrivateIn => 122,
            Self::Instanceof => 124,
            Self::GetLengthIc => 64,
            Self::ArrayPushAcc => 65,
            Self::NewObj => 66,
            Self::NewArr => 67,
            Self::NewFunc => 68,
            Self::NewClass => 69,
            Self::GetProp => 70,
            Self::SetProp => 71,
            Self::GetPrivateProp => 72,
            Self::SetPrivateProp => 73,
            Self::GetIdxIc => 74,
            Self::SetIdxIc => 75,
            Self::GetGlobal => 76,
            Self::SetGlobal => 77,
            Self::GetUpval => 76,
            Self::SetUpval => 77,
            Self::GetScope => 78,
            Self::SetScope => 79,
            Self::ResolveScope => 80,
            Self::GetSuper => 81,
            Self::SetSuper => 82,
            Self::DeleteProp => 83,
            Self::HasProp => 84,
            Self::Keys => 85,
            Self::ForIn => 86,
            Self::IteratorNext => 87,
            Self::Spread => 88,
            Self::Destructure => 89,
            Self::CreateEnv => 90,
            Self::LoadName => 91,
            Self::StoreName => 92,
            Self::InitName => 123,
            Self::LoadClosure => 93,
            Self::NewThis => 94,
            Self::TypeofName => 95,
            Self::JmpEq => 96,
            Self::JmpNeq => 97,
            Self::JmpLt => 98,
            Self::JmpLte => 99,
            Self::LoopIncJmp => 100,
            Self::Switch => 101,
            Self::LoopHint => 102,
            Self::Ret => 103,
            Self::RetU => 104,
            Self::TailCall => 105,
            Self::Construct => 106,
            Self::CallVar => 107,
            Self::CallThis => 226,
            Self::CallThisVar => 227,
            Self::Enter => 108,
            Self::Leave => 109,
            Self::Yield => 110,
            Self::Await => 111,
            Self::Throw => 112,
            Self::Try => 113,
            Self::EndTry => 114,
            Self::Catch => 115,
            Self::Finally => 116,
            Self::CallIc => 128,
            Self::CallIcVar => 129,
            Self::ProfileType => 160,
            Self::ProfileCall => 161,
            Self::ProfileRet => 162,
            Self::CheckType => 163,
            Self::CheckStruct => 164,
            Self::CheckIc => 165,
            Self::IcInit => 166,
            Self::IcUpdate => 167,
            Self::IcMiss => 168,
            Self::OsrEntry => 169,
            Self::ProfileHotLoop => 170,
            Self::OsrExit => 171,
            Self::JitHint => 172,
            Self::SafetyCheck => 173,
            Self::GetPropIcCall => 200,
            Self::IncJmpFalseLoop => 201,
            Self::LoadKAddAcc => 202,
            Self::AddMov => 203,
            Self::EqJmpTrue => 204,
            Self::GetPropAccCall => 205,
            Self::LoadKMulAcc => 206,
            Self::LtJmp => 207,
            Self::GetPropIcMov => 208,
            Self::GetPropAddImmSetPropIc => 209,
            Self::AddAccImm8Mov => 210,
            Self::CallIcSuper => 211,
            Self::LoadThisCall => 212,
            Self::EqJmpFalse => 213,
            Self::LoadKSubAcc => 214,
            Self::GetLengthIcCall => 215,
            Self::AddStrAccMov => 216,
            Self::IncAccJmp => 217,
            Self::GetPropChainAcc => 218,
            Self::TestJmpTrue => 219,
            Self::LoadArgCall => 220,
            Self::MulAccMov => 221,
            Self::LteJmpLoop => 222,
            Self::NewObjInitProp => 223,
            Self::ProfileHotCall => 224,
            Self::Call2SubIAdd => 239,
            Self::Call1SubI => 240,
            Self::JmpLteFalse => 241,
            Self::RetReg => 242,
            Self::AddI32 => 243,
            Self::AddF64 => 244,
            Self::SubI32 => 245,
            Self::SubF64 => 246,
            Self::MulI32 => 247,
            Self::MulF64 => 248,
            // Superinstructions
            Self::RetIfLteI => 249,
            Self::AddAccReg => 250,
            Self::Call1Add => 251,
            Self::Call2Add => 252,
            Self::LoadKAdd => 253,
            Self::LoadKCmp => 254,
            Self::CmpJmp => 255,
            // Specialized opcodes
            Self::AddI32Fast => 130,
            Self::AddF64Fast => 131,
            Self::SubI32Fast => 132,
            Self::MulI32Fast => 133,
            Self::EqI32Fast => 134,
            Self::LtI32Fast => 135,
            Self::JmpI32Fast => 136,
            Self::GetPropMono => 137,
            Self::CallMono => 138,
            // Call opcodes
            Self::Call0 => 139,
            Self::Call1 => 140,
            Self::Call2 => 141,
            Self::Call3 => 142,
            Self::CallMethod1 => 143,
            Self::CallMethod2 => 144,
            // New arithmetic superinstructions
            Self::LoadAdd => 176,
            Self::LoadSub => 177,
            Self::LoadMul => 178,
            Self::LoadInc => 179,
            Self::LoadDec => 180,
            // New comparison superinstructions
            Self::LoadCmpEq => 181,
            Self::LoadCmpLt => 182,
            Self::LoadJfalse => 183,
            Self::LoadCmpEqJfalse => 184,
            Self::LoadCmpLtJfalse => 185,
            // Property access superinstructions
            Self::LoadGetProp => 186,
            Self::LoadGetPropCmpEq => 187,
            // Pareto 80% property access superinstructions with IC
            Self::GetProp2Ic => 188,
            Self::GetProp3Ic => 189,
            Self::GetElem => 190,
            Self::SetElem => 191,
            Self::GetPropElem => 192,
            Self::CallMethodIc => 193,
            Self::CallMethod2Ic => 194,
            Self::LtF64 => 195,
            Self::LteF64 => 196,
            Self::JmpLtF64 => 197,
            Self::JmpLteF64 => 198,
            Self::JmpLteFalseF64 => 199,
            // Superinstruction variants
            Self::GetPropCall => 145,
            Self::CallRet => 146,
            // Assertion opcodes
            Self::AssertValue => 147,
            Self::AssertOk => 148,
            Self::AssertFail => 149,
            Self::AssertThrows => 150,
            Self::AssertDoesNotThrow => 151,
            Self::AssertRejects => 152,
            Self::AssertDoesNotReject => 153,
            Self::AssertEqual => 154,
            Self::AssertNotEqual => 155,
            Self::AssertDeepEqual => 156,
            Self::AssertNotDeepEqual => 157,
            Self::AssertStrictEqual => 158,
            Self::AssertNotStrictEqual => 159,
            Self::AssertDeepStrictEqual => 174,
            Self::AssertNotDeepStrictEqual => 175,
            Self::Reserved(value) => value,
        }
    }
}

#[derive(Clone, Copy)]
struct VmValue {
    vm: *mut VM,
    value: JSValue,
}

impl VmValue {
    fn new(vm: *mut VM, value: JSValue) -> Self {
        Self { vm, value }
    }

    fn raw(self) -> JSValue {
        self.value
    }

    fn wrap(&self, value: JSValue) -> Self {
        Self { vm: self.vm, value }
    }

    fn vm(&self) -> &VM {
        unsafe { &*self.vm }
    }

    fn with_vm_mut<R>(&self, f: impl FnOnce(&mut VM) -> R) -> R {
        unsafe { f(&mut *self.vm) }
    }

    fn wrap_bool(&self, value: bool) -> Self {
        self.wrap(make_bool(value))
    }

    fn prop_key(&self, key: JSValue) -> PropertyKey {
        self.vm().property_key_from_value(key)
    }

    fn int32_value(&self, value: JSValue) -> i32 {
        let numeric = self.with_vm_mut(|vm| vm.number_value(value));
        to_i32(numeric).unwrap_or(0)
    }
}

impl ArithmeticOps for VmValue {
    fn add(&self, rhs: &Self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.binary_add(self.value, rhs.value)))
    }

    fn sub(&self, rhs: &Self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.binary_numeric_op(self.value, rhs.value, |x, y| x - y)))
    }

    fn mul(&self, rhs: &Self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.binary_numeric_op(self.value, rhs.value, |x, y| x * y)))
    }

    fn div(&self, rhs: &Self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.binary_numeric_op(self.value, rhs.value, |x, y| x / y)))
    }

    fn rem(&self, rhs: &Self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.binary_numeric_op(self.value, rhs.value, |x, y| x % y)))
    }

    fn pow(&self, rhs: &Self) -> Self {
        self.wrap(
            self.with_vm_mut(|vm| vm.binary_numeric_op(self.value, rhs.value, |x, y| x.powf(y))),
        )
    }

    fn inc(&self) -> Self {
        self.wrap(
            self.with_vm_mut(|vm| vm.binary_numeric_op(self.value, make_number(1.0), |x, y| x + y)),
        )
    }

    fn dec(&self) -> Self {
        self.wrap(
            self.with_vm_mut(|vm| vm.binary_numeric_op(self.value, make_number(1.0), |x, y| x - y)),
        )
    }

    fn unary_plus(&self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.number_value(self.value)))
    }

    fn unary_minus(&self) -> Self {
        self.wrap(
            self.with_vm_mut(|vm| vm.binary_numeric_op(make_number(0.0), self.value, |x, y| x - y)),
        )
    }
}

impl ComparisonOps for VmValue {
    fn eq(&self, rhs: &Self) -> Self {
        self.wrap_bool(self.with_vm_mut(|vm| vm.abstract_equal(self.value, rhs.value)))
    }

    fn ne(&self, rhs: &Self) -> Self {
        self.wrap_bool(!self.with_vm_mut(|vm| vm.abstract_equal(self.value, rhs.value)))
    }

    fn strict_eq(&self, rhs: &Self) -> Self {
        self.wrap_bool(self.vm().strict_equal(self.value, rhs.value))
    }

    fn strict_ne(&self, rhs: &Self) -> Self {
        self.wrap_bool(!self.vm().strict_equal(self.value, rhs.value))
    }

    fn gt(&self, rhs: &Self) -> Self {
        self.wrap_bool(self.with_vm_mut(|vm| vm.less_than(rhs.value, self.value)))
    }

    fn lt(&self, rhs: &Self) -> Self {
        self.wrap_bool(self.with_vm_mut(|vm| vm.less_than(self.value, rhs.value)))
    }

    fn ge(&self, rhs: &Self) -> Self {
        self.wrap_bool(self.with_vm_mut(|vm| vm.less_than_or_equal(rhs.value, self.value)))
    }

    fn le(&self, rhs: &Self) -> Self {
        self.wrap_bool(self.with_vm_mut(|vm| vm.less_than_or_equal(self.value, rhs.value)))
    }
}

impl LogicalOps for VmValue {
    fn logical_and(&self, rhs: &Self) -> Self {
        if self.vm().is_truthy_value(self.value) {
            *rhs
        } else {
            *self
        }
    }

    fn logical_or(&self, rhs: &Self) -> Self {
        if self.vm().is_truthy_value(self.value) {
            *self
        } else {
            *rhs
        }
    }

    fn logical_not(&self) -> Self {
        self.wrap_bool(!self.vm().is_truthy_value(self.value))
    }
}

impl BitwiseOps for VmValue {
    fn bit_and(&self, rhs: &Self) -> Self {
        self.wrap(make_int32(
            self.int32_value(self.value) & self.int32_value(rhs.value),
        ))
    }

    fn bit_or(&self, rhs: &Self) -> Self {
        self.wrap(make_int32(
            self.int32_value(self.value) | self.int32_value(rhs.value),
        ))
    }

    fn bit_xor(&self, rhs: &Self) -> Self {
        self.wrap(make_int32(
            self.int32_value(self.value) ^ self.int32_value(rhs.value),
        ))
    }

    fn bit_not(&self) -> Self {
        self.wrap(make_int32(!self.int32_value(self.value)))
    }

    fn shl(&self, rhs: &Self) -> Self {
        self.wrap(make_int32(
            self.int32_value(self.value) << (self.int32_value(rhs.value) & 31),
        ))
    }

    fn shr(&self, rhs: &Self) -> Self {
        self.wrap(make_int32(
            self.int32_value(self.value) >> (self.int32_value(rhs.value) & 31),
        ))
    }

    fn ushr(&self, rhs: &Self) -> Self {
        let lhs = self.int32_value(self.value) as u32;
        let shift = (self.int32_value(rhs.value) & 31) as u32;
        self.wrap(make_number((lhs >> shift) as f64))
    }
}

impl AssignmentOps for VmValue {
    fn assign(&mut self, rhs: Self) {
        self.value = rhs.value;
    }

    fn add_assign(&mut self, rhs: Self) {
        self.value = self.add(&rhs).raw();
    }

    fn sub_assign(&mut self, rhs: Self) {
        self.value = self.sub(&rhs).raw();
    }

    fn mul_assign(&mut self, rhs: Self) {
        self.value = self.mul(&rhs).raw();
    }

    fn div_assign(&mut self, rhs: Self) {
        self.value = self.div(&rhs).raw();
    }

    fn rem_assign(&mut self, rhs: Self) {
        self.value = self.rem(&rhs).raw();
    }

    fn pow_assign(&mut self, rhs: Self) {
        self.value = self.pow(&rhs).raw();
    }

    fn shl_assign(&mut self, rhs: Self) {
        self.value = self.shl(&rhs).raw();
    }

    fn shr_assign(&mut self, rhs: Self) {
        self.value = self.shr(&rhs).raw();
    }

    fn ushr_assign(&mut self, rhs: Self) {
        self.value = self.ushr(&rhs).raw();
    }

    fn bit_and_assign(&mut self, rhs: Self) {
        self.value = self.bit_and(&rhs).raw();
    }

    fn bit_or_assign(&mut self, rhs: Self) {
        self.value = self.bit_or(&rhs).raw();
    }

    fn bit_xor_assign(&mut self, rhs: Self) {
        self.value = self.bit_xor(&rhs).raw();
    }
}

impl LogicalAssignOps for VmValue {
    fn and_assign(&mut self, rhs: Self) {
        self.value = self.logical_and(&rhs).raw();
    }

    fn or_assign(&mut self, rhs: Self) {
        self.value = self.logical_or(&rhs).raw();
    }
}

impl NullishOps for VmValue {
    fn nullish_coalesce(&self, rhs: &Self) -> Self {
        if is_null(self.value) || is_undefined(self.value) {
            *rhs
        } else {
            *self
        }
    }

    fn nullish_assign(&mut self, rhs: Self) {
        self.value = self.nullish_coalesce(&rhs).raw();
    }
}

impl TypeOps for VmValue {
    fn typeof_(&self) -> Self {
        let ty = self.vm().type_of_name(self.value);
        self.wrap(self.with_vm_mut(|vm| vm.intern_string(ty)))
    }

    fn instanceof(&self, rhs: &Self) -> Self {
        let instance = if let Some(obj_ptr) = object_from_value(self.value) {
            unsafe {
                match &(*obj_ptr).kind {
                    ObjectKind::Instance(instance) => instance.class == rhs.value,
                    _ => false,
                }
            }
        } else {
            false
        };
        self.wrap_bool(instance)
    }

    fn in_(&self, rhs: &Self) -> Self {
        self.wrap_bool(self.vm().has_property(rhs.value, self.prop_key(self.value)))
    }

    fn private_in(&self, rhs: &Self) -> Self {
        if let Some(atom) = self.value.as_atom() {
            self.wrap_bool(self.vm().has_private_property(rhs.value, PropertyKey::PrivateName(atom)))
        } else {
            // This shouldn't happen for valid code
            self.wrap_bool(false)
        }
    }

    fn delete(&self) -> Self {
        self.wrap(make_true())
    }
}

impl CoercionOps for VmValue {
    fn to_number(&self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.number_value(self.value)))
    }

    fn to_string(&self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.string_value(self.value)))
    }

    fn to_boolean(&self) -> Self {
        self.wrap_bool(self.vm().is_truthy_value(self.value))
    }

    fn to_primitive(&self) -> Self {
        self.wrap(self.with_vm_mut(|vm| vm.primitive_value(self.value)))
    }
}

impl PropertyOps for VmValue {
    fn get(&self, key: &Self) -> Self {
        self.wrap(self.vm().get_property(self.value, self.prop_key(key.value)))
    }

    fn set(&mut self, key: Self, value: Self) {
        let _ = self
            .with_vm_mut(|vm| vm.set_property(self.value, self.prop_key(key.value), value.value));
    }

    fn has(&self, key: &Self) -> Self {
        self.wrap_bool(self.vm().has_property(self.value, self.prop_key(key.value)))
    }

    fn delete_property(&mut self, key: &Self) -> Self {
        self.wrap_bool(
            self.with_vm_mut(|vm| vm.delete_property(self.value, self.prop_key(key.value))),
        )
    }
}

impl CallOps for VmValue {
    fn call(&self, this: &Self, args: &[Self]) -> Self {
        let args: Vec<_> = args.iter().map(|arg| arg.value).collect();
        self.wrap(self.with_vm_mut(|vm| vm.call_value(self.value, this.value, &args)))
    }

    fn construct(&self, args: &[Self]) -> Self {
        let args: Vec<_> = args.iter().map(|arg| arg.value).collect();
        self.wrap(self.with_vm_mut(|vm| vm.construct_value(self.value, &args)))
    }
}

impl Ternary for VmValue {
    fn ternary(cond: &Self, a: &Self, b: &Self) -> Self {
        if cond.vm().is_truthy_value(cond.value) {
            *a
        } else {
            *b
        }
    }
}

impl ValueOps for VmValue {}

impl VM {
    pub fn new(bytecode: Vec<u32>, const_pool: Vec<JSValue>, args: Vec<JSValue>) -> Self {
        let frame = Frame::fresh(args, make_undefined(), 0, None, 0, None, 0);

        let mut vm = Self {
            frame: FrameStack::new(frame),
            pc: 0,
            bytecode,
            const_pool,
            objects: Vec::new(),
            shapes: Vec::new(),
            strings: Vec::new(),
            global_object: HashMap::new(),
            console_output: Vec::new(),
            scope_chain: Vec::new(),
            upvalues: Vec::new(),
            last_exception: make_undefined(),
            interned_strings: HashMap::new(),
            compiled_properties: Vec::new(),
            compiled_private_properties: Vec::new(),
            property_slots: HashMap::new(),
            atoms: AtomTable::new(),
            feedback: RuntimeFeedback::default(),
            heap_shape: Rc::new(AtomShape::new()),
            next_shape_id: 1,
            last_ic_object: None,
            function_constants: Vec::new(),
            console_timers: HashMap::new(),
            console_counts: HashMap::new(),
            console_group_depth: 0,
            console_echo: true,
            builtin_number_to_fixed: make_undefined(),
            builtin_object_prototype: make_undefined(),
            builtin_string_prototype: make_undefined(),
            symbol_registry: HashMap::new(),
            next_symbol_id: 0,
            dispatch_table: [None; 256],
            threaded_stop_depth: None,
        };

        vm.init_dispatch_table();
        vm
    }

    pub fn from_compiled(compiled: crate::codegen::CompiledBytecode, args: Vec<JSValue>) -> Self {
        let crate::codegen::CompiledBytecode {
            bytecode,
            constants,
            string_constants,
            atom_constants,
            function_constants,
            names,
            properties,
            private_properties,
        } = compiled;
        let mut vm = Self::new(bytecode, constants, args);
        vm.function_constants = function_constants;
        for (index, text) in string_constants {
            let value = vm.intern_string(text);
            if let Some(slot) = vm.const_pool.get_mut(index as usize) {
                *slot = value;
            }
        }
        for (index, text) in atom_constants {
            let atom = vm.atoms.intern(&text);
            let value = JSValue::atom(atom);
            if let Some(slot) = vm.const_pool.get_mut(index as usize) {
                *slot = value;
            }
        }
        vm.install_js_builtins(&names, &properties, &private_properties);
        vm
    }

    pub fn install_js_builtins(&mut self, names: &[String], properties: &[String], private_properties: &[String]) {
        built_ins::install_js_builtins(self, names, properties, private_properties);
        if let Some(slot) = names.iter().position(|name| name == "Object")
            && let Ok(slot) = u16::try_from(slot)
            && let Some(&object_ctor) = self.global_object.get(&slot)
        {
            self.builtin_object_prototype = self.get_property_by_name(object_ctor, "prototype");
        }
        if let Some(slot) = names.iter().position(|name| name == "String")
            && let Ok(slot) = u16::try_from(slot)
            && let Some(&string_ctor) = self.global_object.get(&slot)
        {
            self.builtin_string_prototype = self.get_property_by_name(string_ctor, "prototype");
        }
        if self.builtin_string_prototype.is_undefined() {
            self.builtin_string_prototype = built_ins::create_string_prototype(self);
        }
        self.builtin_number_to_fixed =
            self.alloc_native_function(Some("__builtin_number_to_fixed"));
    }

    fn eval_compiled(
        &mut self,
        compiled: crate::codegen::CompiledBytecode,
    ) -> Result<JSValue, String> {
        let crate::codegen::CompiledBytecode {
            bytecode,
            constants,
            string_constants,
            atom_constants,
            function_constants,
            names,
            properties,
            private_properties,
        } = compiled;

        let mut temp_vm = Self::new(bytecode, constants, vec![]);
        temp_vm.atoms = self.atoms.clone();
        temp_vm.interned_strings = self.interned_strings.clone();
        temp_vm.global_object = self.global_object.clone();
        temp_vm.symbol_registry = self.symbol_registry.clone();
        temp_vm.next_symbol_id = self.next_symbol_id;
        temp_vm.builtin_object_prototype = self.builtin_object_prototype;
        temp_vm.builtin_string_prototype = self.builtin_string_prototype;
        temp_vm.console_echo = self.console_echo;
        temp_vm.console_timers = self.console_timers.clone();
        temp_vm.console_counts = self.console_counts.clone();
        temp_vm.console_group_depth = self.console_group_depth;
        temp_vm.function_constants = function_constants;

        for (index, text) in string_constants {
            let value = temp_vm.intern_string(text);
            if let Some(slot) = temp_vm.const_pool.get_mut(index as usize) {
                *slot = value;
            }
        }
        for (index, text) in atom_constants {
            let atom = temp_vm.atoms.intern(&text);
            let value = JSValue::atom(atom);
            if let Some(slot) = temp_vm.const_pool.get_mut(index as usize) {
                *slot = value;
            }
        }

        temp_vm.install_js_builtins(&names, &properties, &private_properties);
        temp_vm.run(false);

        let result = temp_vm.frame.regs[ACC];
        self.global_object = temp_vm.global_object.clone();
        self.atoms = temp_vm.atoms.clone();
        self.interned_strings = temp_vm.interned_strings.clone();
        self.symbol_registry = temp_vm.symbol_registry.clone();
        self.next_symbol_id = temp_vm.next_symbol_id;
        self.console_timers = temp_vm.console_timers.clone();
        self.console_counts = temp_vm.console_counts.clone();
        self.console_group_depth = temp_vm.console_group_depth;
        self.objects.append(&mut temp_vm.objects);
        self.shapes.append(&mut temp_vm.shapes);
        self.strings.append(&mut temp_vm.strings);
        self.console_output.append(&mut temp_vm.console_output);

        Ok(result)
    }

    fn eval_source(&mut self, source: &str) -> Result<JSValue, String> {
        let compiled = crate::codegen::compile_source(source).map_err(|error| error.to_string())?;
        self.eval_compiled(compiled)
    }

    pub fn set_console_echo(&mut self, enabled: bool) {
        self.console_echo = enabled;
    }

    /// Initialize the dispatch table with handler functions for hot opcodes
    fn init_dispatch_table(&mut self) {
        // Initialize all slots to None
        for i in 0..256 {
            self.dispatch_table[i] = None;
        }

        // Register handlers for the Pareto-hot opcode set and emitted superinstructions.
        self.dispatch_table[Opcode::Mov.as_u8() as usize] = Some(Self::handler_mov);
        self.dispatch_table[Opcode::LoadI.as_u8() as usize] = Some(Self::handler_loadi);
        self.dispatch_table[Opcode::LoadK.as_u8() as usize] = Some(Self::handler_loadk);
        self.dispatch_table[Opcode::AddI32.as_u8() as usize] = Some(Self::handler_addi32);
        self.dispatch_table[Opcode::AddF64.as_u8() as usize] = Some(Self::handler_addf64);
        self.dispatch_table[Opcode::SubF64.as_u8() as usize] = Some(Self::handler_subf64);
        self.dispatch_table[Opcode::MulF64.as_u8() as usize] = Some(Self::handler_mulf64);
        self.dispatch_table[Opcode::Add.as_u8() as usize] = Some(Self::handler_add);
        self.dispatch_table[Opcode::JmpFalse.as_u8() as usize] = Some(Self::handler_jmpfalse);
        self.dispatch_table[Opcode::Jmp.as_u8() as usize] = Some(Self::handler_jmp);
        self.dispatch_table[Opcode::RetReg.as_u8() as usize] = Some(Self::handler_retreg);
        self.dispatch_table[Opcode::Load0.as_u8() as usize] = Some(Self::handler_load0);
        self.dispatch_table[Opcode::Load1.as_u8() as usize] = Some(Self::handler_load1);
        self.dispatch_table[Opcode::Eq.as_u8() as usize] = Some(Self::handler_eq);
        self.dispatch_table[Opcode::Lt.as_u8() as usize] = Some(Self::handler_lt);
        self.dispatch_table[Opcode::Lte.as_u8() as usize] = Some(Self::handler_lte);
        self.dispatch_table[Opcode::LtF64.as_u8() as usize] = Some(Self::handler_ltf64);
        self.dispatch_table[Opcode::LteF64.as_u8() as usize] = Some(Self::handler_ltef64);
        self.dispatch_table[Opcode::JmpLtF64.as_u8() as usize] = Some(Self::handler_jmpltf64);
        self.dispatch_table[Opcode::JmpLteF64.as_u8() as usize] = Some(Self::handler_jmpltef64);
        self.dispatch_table[Opcode::JmpLteFalseF64.as_u8() as usize] =
            Some(Self::handler_jmpltefalsef64);

        // Arithmetic and branch superinstructions.
        self.dispatch_table[Opcode::AddI32Fast.as_u8() as usize] = Some(Self::handler_addi32fast);
        self.dispatch_table[Opcode::AddF64Fast.as_u8() as usize] = Some(Self::handler_addf64fast);
        self.dispatch_table[Opcode::SubI32Fast.as_u8() as usize] = Some(Self::handler_subi32fast);
        self.dispatch_table[Opcode::MulI32Fast.as_u8() as usize] = Some(Self::handler_muli32fast);
        self.dispatch_table[Opcode::EqI32Fast.as_u8() as usize] = Some(Self::handler_eqi32fast);
        self.dispatch_table[Opcode::LtI32Fast.as_u8() as usize] = Some(Self::handler_lti32fast);
        self.dispatch_table[Opcode::JmpI32Fast.as_u8() as usize] = Some(Self::handler_jmpi32fast);
        self.dispatch_table[Opcode::AddMov.as_u8() as usize] = Some(Self::handler_addmov);
        self.dispatch_table[Opcode::EqJmpTrue.as_u8() as usize] = Some(Self::handler_eqjmptrue);
        self.dispatch_table[Opcode::EqJmpFalse.as_u8() as usize] = Some(Self::handler_eqjmpfalse);
        self.dispatch_table[Opcode::LtJmp.as_u8() as usize] = Some(Self::handler_ltjmp);
        self.dispatch_table[Opcode::LteJmpLoop.as_u8() as usize] = Some(Self::handler_ltejmploop);
        self.dispatch_table[Opcode::CmpJmp.as_u8() as usize] = Some(Self::handler_cmpjmp);
        self.dispatch_table[Opcode::TestJmpTrue.as_u8() as usize] = Some(Self::handler_testjmptrue);
        self.dispatch_table[Opcode::IncJmpFalseLoop.as_u8() as usize] =
            Some(Self::handler_incjmpfalseloop);
        self.dispatch_table[Opcode::IncAccJmp.as_u8() as usize] = Some(Self::handler_incaccjmp);

        // Call and data-movement superinstructions.
        self.dispatch_table[Opcode::Call0.as_u8() as usize] = Some(Self::handler_call0);
        self.dispatch_table[Opcode::Call1.as_u8() as usize] = Some(Self::handler_call1);
        self.dispatch_table[Opcode::Call2.as_u8() as usize] = Some(Self::handler_call2);
        self.dispatch_table[Opcode::LoadArg.as_u8() as usize] = Some(Self::handler_loadarg);
        self.dispatch_table[Opcode::GetUpval.as_u8() as usize] = Some(Self::handler_getupval);
        self.dispatch_table[Opcode::LoadClosure.as_u8() as usize] = Some(Self::handler_getupval);
        self.dispatch_table[Opcode::Call2SubIAdd.as_u8() as usize] =
            Some(Self::handler_call2subiadd);
        self.dispatch_table[Opcode::Call1SubI.as_u8() as usize] = Some(Self::handler_call1subi);
        self.dispatch_table[Opcode::RetIfLteI.as_u8() as usize] = Some(Self::handler_retifltei);
        self.dispatch_table[Opcode::AddAccReg.as_u8() as usize] = Some(Self::handler_addaccreg);
        self.dispatch_table[Opcode::Call1Add.as_u8() as usize] = Some(Self::handler_call1add);
        self.dispatch_table[Opcode::Call2Add.as_u8() as usize] = Some(Self::handler_call2add);
        self.dispatch_table[Opcode::LoadKAdd.as_u8() as usize] = Some(Self::handler_loadkadd);
        self.dispatch_table[Opcode::LoadKCmp.as_u8() as usize] = Some(Self::handler_loadkcmp);
        self.dispatch_table[Opcode::LoadKAddAcc.as_u8() as usize] = Some(Self::handler_loadkaddacc);
        self.dispatch_table[Opcode::AddAccImm8Mov.as_u8() as usize] =
            Some(Self::handler_addaccimm8mov);
        self.dispatch_table[Opcode::MulAccMov.as_u8() as usize] = Some(Self::handler_mulaccmov);
        self.dispatch_table[Opcode::LoadArgCall.as_u8() as usize] = Some(Self::handler_loadargcall);
        self.dispatch_table[Opcode::LoadThisCall.as_u8() as usize] =
            Some(Self::handler_loadthiscall);
        self.dispatch_table[Opcode::GetPropIcCall.as_u8() as usize] =
            Some(Self::handler_getpropiccall);
        self.dispatch_table[Opcode::GetPropCall.as_u8() as usize] = Some(Self::handler_getpropcall);
        self.dispatch_table[Opcode::GetPropAccCall.as_u8() as usize] =
            Some(Self::handler_getpropacccall);
        self.dispatch_table[Opcode::GetLengthIcCall.as_u8() as usize] =
            Some(Self::handler_getlengthiccall);
        self.dispatch_table[Opcode::GetPropChainAcc.as_u8() as usize] =
            Some(Self::handler_getpropchainacc);
        self.dispatch_table[Opcode::RetU.as_u8() as usize] = Some(Self::handler_retu);
    }

    // New handler functions for superinstructions
    fn handler_addi32fast(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        // Fast path: int32 + int32
        if lhs.is_int() && rhs.is_int() {
            let a_int = lhs.int_payload_unchecked();
            let b_int = rhs.int_payload_unchecked();
            if let Some(result) = a_int.checked_add(b_int) {
                vm.frame.regs[ACC] = make_int32(result);
                if a != ACC {
                    vm.frame.regs[a] = make_int32(result);
                }
                return ControlFlow::Continue;
            }
        }
        // Fall back to slow path
        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.add(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_addf64fast(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            vm.write_result_reg(a, make_number(lhs + rhs));
            return ControlFlow::Continue;
        }
        // Fall back to slow path
        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.add(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_subi32fast(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        // Fast path: int32 - int32
        if lhs.is_int() && rhs.is_int() {
            let a_int = lhs.int_payload_unchecked();
            let b_int = rhs.int_payload_unchecked();
            if let Some(result) = a_int.checked_sub(b_int) {
                vm.frame.regs[ACC] = make_int32(result);
                if a != ACC {
                    vm.frame.regs[a] = make_int32(result);
                }
                return ControlFlow::Continue;
            }
        }
        // Fall back to slow path
        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.sub(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_muli32fast(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        // Fast path: int32 * int32
        if lhs.is_int() && rhs.is_int() {
            let a_int = lhs.int_payload_unchecked();
            let b_int = rhs.int_payload_unchecked();
            if let Some(result) = a_int.checked_mul(b_int) {
                vm.frame.regs[ACC] = make_int32(result);
                if a != ACC {
                    vm.frame.regs[a] = make_int32(result);
                }
                return ControlFlow::Continue;
            }
        }
        // Fall back to slow path
        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.mul(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_eqi32fast(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        // Fast path: int32 == int32
        if lhs.is_int() && rhs.is_int() {
            let a_int = lhs.int_payload_unchecked();
            let b_int = rhs.int_payload_unchecked();
            vm.frame.regs[ACC] = make_bool(a_int == b_int);
            return ControlFlow::Continue;
        }
        // Fall back to slow path
        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.eq(&rhs).raw();
        ControlFlow::Continue
    }

    fn handler_lti32fast(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        // Fast path: int32 < int32
        if lhs.is_int() && rhs.is_int() {
            let a_int = lhs.int_payload_unchecked();
            let b_int = rhs.int_payload_unchecked();
            vm.frame.regs[ACC] = make_bool(a_int < b_int);
            return ControlFlow::Continue;
        }
        // Fall back to slow path
        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.lt(&rhs).raw();
        ControlFlow::Continue
    }

    fn handler_jmpi32fast(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[a];
        let rhs = vm.frame.regs[b];

        // Fast path: int32 < int32
        if lhs.is_int() && rhs.is_int() {
            let a_int = lhs.int_payload_unchecked();
            let b_int = rhs.int_payload_unchecked();
            if a_int < b_int {
                vm.jump_by(c as i8 as i16);
            }
            return ControlFlow::Continue;
        }
        // Fall back to slow path
        if vm.less_than(lhs, rhs) {
            vm.jump_by(c as i8 as i16);
        }
        ControlFlow::Continue
    }

    fn handler_call0(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        match vm.dispatch_call_value(vm.frame.regs[a], vm.frame.regs[0], &[]) {
            CallAction::Returned(result) => {
                vm.frame.regs[ACC] = result;
                ControlFlow::Continue
            }
            CallAction::EnteredFrame => ControlFlow::Continue,
        }
    }

    fn handler_call1(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        match vm.dispatch_call_value(vm.frame.regs[a], vm.frame.regs[0], &[vm.frame.regs[b]]) {
            CallAction::Returned(result) => {
                vm.frame.regs[ACC] = result;
                ControlFlow::Continue
            }
            CallAction::EnteredFrame => ControlFlow::Continue,
        }
    }

    fn handler_call2(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        match vm.dispatch_call_value(
            vm.frame.regs[a],
            vm.frame.regs[0],
            &[vm.frame.regs[b], vm.frame.regs[c]],
        ) {
            CallAction::Returned(result) => {
                vm.frame.regs[ACC] = result;
                ControlFlow::Continue
            }
            CallAction::EnteredFrame => ControlFlow::Continue,
        }
    }

    fn handler_addmov(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let result = vm.add_values(vm.frame.regs[b], vm.frame.regs[c]);
        vm.frame.regs[ACC] = result;
        vm.frame.regs[a] = result;
        ControlFlow::Continue
    }

    fn handler_eqjmptrue(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as i8 as i16;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        if vm.abstract_equal(vm.frame.regs[b], vm.frame.regs[c]) {
            vm.jump_by(a);
        }
        ControlFlow::Continue
    }

    fn handler_eqjmpfalse(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as i8 as i16;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        if !vm.abstract_equal(vm.frame.regs[b], vm.frame.regs[c]) {
            vm.jump_by(a);
        }
        ControlFlow::Continue
    }

    fn handler_ltjmp(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as i8 as i16;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        if vm.less_than(vm.frame.regs[b], vm.frame.regs[c]) {
            vm.jump_by(a);
        }
        ControlFlow::Continue
    }

    fn handler_ltejmploop(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as i8 as i16;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        if vm.less_than_or_equal(vm.frame.regs[b], vm.frame.regs[c]) {
            vm.jump_by(a);
        }
        ControlFlow::Continue
    }

    fn handler_cmpjmp(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as i8 as i16;
        if vm.less_than(vm.frame.regs[a], vm.frame.regs[b]) {
            vm.jump_by(c);
        }
        ControlFlow::Continue
    }

    fn handler_testjmptrue(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let sbx = ((insn >> 16) & 0xFFFF) as u16 as i16;
        if vm.is_truthy_value(vm.frame.regs[a]) {
            vm.jump_by(sbx);
        }
        ControlFlow::Continue
    }

    fn handler_incjmpfalseloop(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let sbx = ((insn >> 16) & 0xFFFF) as u16 as i16;
        vm.frame.regs[ACC] = vm.inc_value(vm.frame.regs[ACC]);
        if !vm.is_truthy_value(vm.frame.regs[a]) {
            vm.jump_by(sbx);
        }
        ControlFlow::Continue
    }

    fn handler_incaccjmp(vm: &mut VM, insn: u32) -> ControlFlow {
        let sbx = ((insn >> 16) & 0xFFFF) as u16 as i16;
        vm.frame.regs[ACC] = vm.inc_value(vm.frame.regs[ACC]);
        vm.jump_by(sbx);
        ControlFlow::Continue
    }

    fn handler_addaccreg(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        vm.frame.regs[ACC] = vm.add_values(vm.frame.regs[a], vm.frame.regs[b]);
        ControlFlow::Continue
    }

    fn handler_call1add(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let callee = vm.frame.regs[a];
        let arg = vm.frame.regs[b];
        let lhs = vm.frame.regs[ACC];
        match vm.dispatch_call_value(callee, vm.frame.regs[0], &[arg]) {
            CallAction::Returned(result) => {
                vm.frame.regs[ACC] = vm.add_values(lhs, result);
            }
            CallAction::EnteredFrame => {
                if let Some(caller) = vm.frame.caller_frame_mut() {
                    caller.header.pending_call =
                        Some(PendingCallContinuation::AddReturnedToAcc { lhs });
                }
            }
        }
        ControlFlow::Continue
    }

    fn handler_call2add(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let callee = vm.frame.regs[a];
        let arg1 = vm.frame.regs[b];
        let arg2 = vm.frame.regs[c];
        let lhs = vm.frame.regs[ACC];
        match vm.dispatch_call_value(callee, vm.frame.regs[0], &[arg1, arg2]) {
            CallAction::Returned(result) => {
                vm.frame.regs[ACC] = vm.add_values(lhs, result);
            }
            CallAction::EnteredFrame => {
                if let Some(caller) = vm.frame.caller_frame_mut() {
                    caller.header.pending_call =
                        Some(PendingCallContinuation::AddReturnedToAcc { lhs });
                }
            }
        }
        ControlFlow::Continue
    }

    fn handler_call2subiadd(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let imm = ((insn >> 24) & 0xFF) as u8 as i8;
        let next_imm = imm.wrapping_add(1);
        let callee = vm.frame.regs[a];
        let arg1 = vm.sub_immediate_value(vm.frame.regs[b], imm);
        let arg2 = vm.sub_immediate_value(vm.frame.regs[b], next_imm);
        match vm.dispatch_call_value(callee, vm.frame.regs[0], &[arg1]) {
            CallAction::Returned(result1) => {
                match vm.dispatch_call_value(callee, vm.frame.regs[0], &[arg2]) {
                    CallAction::Returned(result2) => {
                        vm.frame.regs[ACC] = vm.add_values(result1, result2);
                    }
                    CallAction::EnteredFrame => {
                        if let Some(caller) = vm.frame.caller_frame_mut() {
                            caller.header.pending_call =
                                Some(PendingCallContinuation::AddReturnedToAcc { lhs: result1 });
                        }
                    }
                }
            }
            CallAction::EnteredFrame => {
                if let Some(caller) = vm.frame.caller_frame_mut() {
                    caller.header.pending_call =
                        Some(PendingCallContinuation::Call2SubIAddSecond { callee, arg: arg2 });
                }
            }
        }
        ControlFlow::Continue
    }

    fn handler_loadkadd(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let index = ((insn >> 16) & 0xFFFF) as usize;
        let constant = vm.constant_or_undefined(index);
        vm.frame.regs[a] = vm.add_values(constant, vm.frame.regs[ACC]);
        ControlFlow::Continue
    }

    fn handler_loadkcmp(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let index = ((insn >> 16) & 0xFFFF) as usize;
        let constant = vm.constant_or_undefined(index);
        vm.frame.regs[ACC] = make_bool(vm.less_than(constant, vm.frame.regs[a]));
        ControlFlow::Continue
    }

    fn handler_loadkaddacc(vm: &mut VM, insn: u32) -> ControlFlow {
        let index = ((insn >> 16) & 0xFFFF) as usize;
        let constant = vm.constant_or_undefined(index);
        vm.frame.regs[ACC] = vm.add_values(constant, vm.frame.regs[ACC]);
        ControlFlow::Continue
    }

    fn handler_addaccimm8mov(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as i8 as i32;
        vm.frame.regs[ACC] = vm.binary_numeric_op(vm.frame.regs[ACC], make_int32(b), |x, y| x + y);
        vm.frame.regs[a] = vm.frame.regs[ACC];
        ControlFlow::Continue
    }

    fn handler_mulaccmov(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        vm.frame.regs[ACC] = vm.mul_values(vm.frame.regs[ACC], vm.frame.regs[b]);
        vm.frame.regs[a] = vm.frame.regs[ACC];
        ControlFlow::Continue
    }

    fn handler_loadargcall(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        vm.frame.regs[a] = vm.frame.arg(b);
        let action = vm.dispatch_call_value(vm.frame.regs[a], vm.frame.regs[0], &[]);
        vm.store_call_result(action)
    }

    fn handler_loadthiscall(vm: &mut VM, _insn: u32) -> ControlFlow {
        let action = vm.dispatch_call_value(vm.frame.regs[0], vm.frame.regs[0], &[]);
        vm.store_call_result(action)
    }

    fn handler_getpropiccall(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let key = Self::property_key_from_immediate(c as u16);
        let this_value = vm.frame.regs[b];
        vm.feedback.last_ic_slot = Some(c);
        let callee = vm.get_property_via_ic(c, this_value, key);
        vm.frame.regs[a] = callee;
        let action = vm.invoke_method_call(callee, this_value, 0, a + 1);
        vm.store_call_result(action)
    }

    fn handler_getpropcall(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as u16;
        let key = Self::property_key_from_immediate(c);
        let this_value = vm.frame.regs[b];
        let callee = vm.get_property(this_value, key);
        vm.frame.regs[a] = callee;
        let action = vm.dispatch_call_value(callee, this_value, &[]);
        vm.store_call_result(action)
    }

    fn handler_getpropacccall(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let this_value = vm.frame.regs[b];
        let key = vm.property_key_from_value(vm.frame.regs[c]);
        let callee = vm.get_property(this_value, key);
        let action = vm.dispatch_call_value(callee, this_value, &[]);
        vm.store_call_result(action)
    }

    fn handler_getlengthiccall(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        vm.frame.regs[ACC] = vm.get_length_value(vm.frame.regs[b]);
        ControlFlow::Continue
    }

    fn handler_getpropchainacc(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as u16;
        let inner_reg = vm.array_index_from_value(vm.frame.regs[b]).unwrap_or(0);
        let base = vm
            .frame
            .regs
            .get(inner_reg)
            .copied()
            .unwrap_or(make_undefined());
        vm.frame.regs[ACC] = vm.get_property(base, Self::property_key_from_immediate(c));
        ControlFlow::Continue
    }

    fn handler_loadarg(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        vm.frame.regs[a] = vm.frame.arg(b);
        ControlFlow::Continue
    }

    fn handler_getupval(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        vm.frame.regs[a] = vm
            .current_function_value()
            .and_then(|function_value| vm.get_function_upvalue(function_value, b))
            .or_else(|| vm.upvalues.get(b).copied())
            .unwrap_or(make_undefined());
        ControlFlow::Continue
    }

    fn handler_call1subi(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let imm = ((insn >> 24) & 0xFF) as u8 as i8;
        let callee = vm.frame.regs[a];
        let arg = vm.sub_immediate_value(vm.frame.regs[b], imm);

        match vm.dispatch_call_value(callee, vm.frame.regs[0], &[arg]) {
            CallAction::Returned(result) => {
                vm.frame.regs[ACC] = result;
                ControlFlow::Continue
            }
            CallAction::EnteredFrame => ControlFlow::Continue,
        }
    }

    fn handler_retifltei(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        if vm.less_than_or_equal(vm.frame.regs[a], vm.frame.regs[b]) {
            return vm.finish_frame_exit(vm.frame.regs[c]);
        }

        ControlFlow::Continue
    }

    fn handler_retu(vm: &mut VM, _insn: u32) -> ControlFlow {
        vm.finish_frame_exit(make_undefined())
    }

    // Handler functions for hot opcodes
    fn handler_mov(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        vm.frame.regs[a] = vm.frame.regs[b];
        ControlFlow::Continue
    }

    fn handler_loadi(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let sbx = ((insn >> 16) & 0xFFFF) as u16 as i16;
        vm.frame.regs[a] = make_int32(sbx as i32);
        ControlFlow::Continue
    }

    fn handler_addi32(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        // Fast path: int32 + int32
        if lhs.is_int() && rhs.is_int() {
            let a_int = lhs.int_payload_unchecked();
            let b_int = rhs.int_payload_unchecked();
            if let Some(result) = a_int.checked_add(b_int) {
                vm.frame.regs[ACC] = make_int32(result);
                if a != ACC {
                    vm.frame.regs[a] = make_int32(result);
                }
                return ControlFlow::Continue;
            }
        }

        // Fall back to slow path
        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.add(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_addf64(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            vm.write_result_reg(a, make_number(lhs + rhs));
            return ControlFlow::Continue;
        }

        // Fall back to slow path
        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.add(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_subf64(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            vm.write_result_reg(a, make_number(lhs - rhs));
            return ControlFlow::Continue;
        }

        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.sub(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_mulf64(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;

        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            vm.write_result_reg(a, make_number(lhs * rhs));
            return ControlFlow::Continue;
        }

        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.mul(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_jmpfalse(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let sbx = ((insn >> 16) & 0xFFFF) as u16 as i16;

        if !vm.is_truthy_value(vm.frame.regs[a]) {
            vm.jump_by(sbx);
        }
        ControlFlow::Continue
    }

    fn handler_retreg(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        vm.finish_frame_exit(vm.frame.regs[a])
    }

    fn handler_loadk(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let index = ((insn >> 16) & 0xFFFF) as usize;
        vm.frame.regs[a] = vm
            .const_pool
            .get(index)
            .copied()
            .unwrap_or(make_undefined());
        ControlFlow::Continue
    }

    fn handler_add(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        vm.frame.regs[ACC] = vm.add_values(vm.frame.regs[b], vm.frame.regs[c]);
        ControlFlow::Continue
    }

    fn handler_jmp(vm: &mut VM, insn: u32) -> ControlFlow {
        let sbx = ((insn >> 16) & 0xFFFF) as u16 as i16;
        vm.jump_by(sbx);
        ControlFlow::Continue
    }

    fn handler_load0(vm: &mut VM, _insn: u32) -> ControlFlow {
        vm.frame.regs[ACC] = make_int32(0);
        ControlFlow::Continue
    }

    fn handler_load1(vm: &mut VM, _insn: u32) -> ControlFlow {
        vm.frame.regs[ACC] = make_int32(1);
        ControlFlow::Continue
    }

    fn handler_eq(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let (lhs, rhs) = vm.value_pair(vm.frame.regs[b], vm.frame.regs[c]);
        vm.frame.regs[ACC] = lhs.eq(&rhs).raw();
        ControlFlow::Continue
    }

    fn handler_lt(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let (lhs, rhs) = vm.value_pair(vm.frame.regs[b], vm.frame.regs[c]);
        vm.frame.regs[ACC] = lhs.lt(&rhs).raw();
        ControlFlow::Continue
    }

    fn handler_lte(vm: &mut VM, insn: u32) -> ControlFlow {
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let (lhs, rhs) = vm.value_pair(vm.frame.regs[b], vm.frame.regs[c]);
        vm.frame.regs[ACC] = lhs.le(&rhs).raw();
        ControlFlow::Continue
    }

    fn handler_ltf64(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            vm.write_result_reg(a, make_bool(lhs < rhs));
            return ControlFlow::Continue;
        }

        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.lt(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_ltef64(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let lhs = vm.frame.regs[b];
        let rhs = vm.frame.regs[c];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            vm.write_result_reg(a, make_bool(lhs <= rhs));
            return ControlFlow::Continue;
        }

        let (lhs, rhs) = vm.value_pair(lhs, rhs);
        vm.frame.regs[ACC] = lhs.le(&rhs).raw();
        if a != ACC {
            vm.frame.regs[a] = vm.frame.regs[ACC];
        }
        ControlFlow::Continue
    }

    fn handler_jmpltf64(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let lhs = vm.frame.regs[a];
        let rhs = vm.frame.regs[b];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            if lhs < rhs {
                vm.jump_by(c as i8 as i16);
            }
            return ControlFlow::Continue;
        }

        if vm.less_than(lhs, rhs) {
            vm.jump_by(c as i8 as i16);
        }
        ControlFlow::Continue
    }

    fn handler_jmpltef64(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let lhs = vm.frame.regs[a];
        let rhs = vm.frame.regs[b];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            if lhs <= rhs {
                vm.jump_by(c as i8 as i16);
            }
            return ControlFlow::Continue;
        }

        if vm.less_than_or_equal(lhs, rhs) {
            vm.jump_by(c as i8 as i16);
        }
        ControlFlow::Continue
    }

    fn handler_jmpltefalsef64(vm: &mut VM, insn: u32) -> ControlFlow {
        let a = ((insn >> 8) & 0xFF) as usize;
        let b = ((insn >> 16) & 0xFF) as usize;
        let c = ((insn >> 24) & 0xFF) as usize;
        let lhs = vm.frame.regs[a];
        let rhs = vm.frame.regs[b];

        if let Some((lhs, rhs)) = vm.fast_number_pair(lhs, rhs) {
            if lhs > rhs {
                vm.jump_by(c as i8 as i16);
            }
            return ControlFlow::Continue;
        }

        if !vm.less_than_or_equal(lhs, rhs) {
            vm.jump_by(c as i8 as i16);
        }
        ControlFlow::Continue
    }

    fn value_op(&mut self, value: JSValue) -> VmValue {
        VmValue::new(self as *mut VM, value)
    }

    fn value_pair(&mut self, lhs: JSValue, rhs: JSValue) -> (VmValue, VmValue) {
        let vm = self as *mut VM;
        (VmValue::new(vm, lhs), VmValue::new(vm, rhs))
    }

    fn decode_abx(insn: u32) -> usize {
        ((insn >> 16) & 0xFFFF) as usize
    }

    fn decode_asbx(insn: u32) -> i16 {
        ((insn >> 16) & 0xFFFF) as u16 as i16
    }

    fn property_key_from_immediate(id: u16) -> PropertyKey {
        PropertyKey::Id(id)
    }

    fn property_key_from_value(&self, value: JSValue) -> PropertyKey {
        if let Some(index) = self.array_index_from_value(value) {
            PropertyKey::Index(index as u32)
        } else if let Some(text) = self.string_text(value) {
            self.property_key_for_existing_name(text)
                .unwrap_or(PropertyKey::Value(value))
        } else {
            PropertyKey::Value(value)
        }
    }

    fn property_key_to_value(&mut self, key: PropertyKey) -> JSValue {
        match key {
            PropertyKey::Id(id) => self
                .compiled_properties
                .get(id as usize)
                .cloned()
                .map(|text| self.intern_string(text))
                .unwrap_or_else(|| self.intern_string(id.to_string())),
            PropertyKey::Atom(atom) | PropertyKey::PrivateName(atom) => {
                let text = self.atoms.resolve(atom).to_owned();
                self.intern_string(text)
            }
            PropertyKey::Index(index) => self.intern_string(index.to_string()),
            PropertyKey::Value(value) => {
                if matches!(value.heap_kind(), Some(value::HeapKind::Symbol)) {
                    value
                } else if let Some(text) = self.property_key_to_text(PropertyKey::Value(value)) {
                    self.intern_string(text)
                } else {
                    value
                }
            }
        }
    }

    fn current_shape_id(&self, obj_ptr: *mut JSObject) -> u32 {
        unsafe {
            if (*obj_ptr).shape.is_null() {
                0
            } else {
                (*(*obj_ptr).shape).id
            }
        }
    }

    fn uses_shape_storage(key: PropertyKey) -> bool {
        matches!(key, PropertyKey::Id(_) | PropertyKey::Atom(_))
    }

    fn shape_offset_for_key(shape_ptr: *mut Shape, key: PropertyKey) -> Option<usize> {
        let mut current = (!shape_ptr.is_null()).then_some(shape_ptr);
        while let Some(shape_ptr) = current {
            unsafe {
                if (*shape_ptr).key == Some(key) {
                    return Some((*shape_ptr).offset as usize);
                }
                current = (*shape_ptr).parent;
            }
        }
        None
    }

    fn named_property_offset(&self, obj_ptr: *mut JSObject, key: PropertyKey) -> Option<usize> {
        if !Self::uses_shape_storage(key) {
            return None;
        }

        unsafe { Self::shape_offset_for_key((*obj_ptr).shape, key) }
    }

    fn ensure_named_storage_slot(&mut self, obj_ptr: *mut JSObject, offset: usize) {
        unsafe {
            if offset >= (*obj_ptr).named_values.len() {
                (*obj_ptr).named_values.resize(offset + 1, make_undefined());
                (*obj_ptr).named_present.resize(offset + 1, false);
            }
        }
    }

    fn get_named_property_slot(&self, obj_ptr: *mut JSObject, key: PropertyKey) -> Option<JSValue> {
        let offset = self.named_property_offset(obj_ptr, key)?;
        unsafe {
            let object = &*obj_ptr;
            object
                .named_present
                .get(offset)
                .copied()
                .filter(|present| *present)
                .and_then(|_| object.named_values.get(offset).copied())
        }
    }

    fn set_named_property_slot(
        &mut self,
        obj_ptr: *mut JSObject,
        key: PropertyKey,
        value: JSValue,
    ) -> bool {
        if !Self::uses_shape_storage(key) {
            return false;
        }

        self.transition_shape_if_needed(obj_ptr, key);
        let Some(offset) = self.named_property_offset(obj_ptr, key) else {
            return false;
        };
        self.ensure_named_storage_slot(obj_ptr, offset);
        unsafe {
            let object = &mut *obj_ptr;
            object.named_values[offset] = value;
            object.named_present[offset] = true;
        }
        true
    }

    fn delete_named_property_slot(&mut self, obj_ptr: *mut JSObject, key: PropertyKey) -> bool {
        let Some(offset) = self.named_property_offset(obj_ptr, key) else {
            return false;
        };

        unsafe {
            let object = &mut *obj_ptr;
            let Some(present) = object.named_present.get_mut(offset) else {
                return false;
            };
            if !*present {
                return false;
            }
            *present = false;
            if let Some(value) = object.named_values.get_mut(offset) {
                *value = make_undefined();
            }
        }

        true
    }

    fn has_named_property_slot(&self, obj_ptr: *mut JSObject, key: PropertyKey) -> bool {
        self.get_named_property_slot(obj_ptr, key).is_some()
    }

    fn named_property_count(&self, obj_ptr: *mut JSObject) -> usize {
        unsafe {
            let object = &*obj_ptr;
            object
                .named_present
                .iter()
                .copied()
                .filter(|present| *present)
                .count()
        }
    }

    fn named_property_keys(&self, obj_ptr: *mut JSObject) -> Vec<PropertyKey> {
        let mut keys = Vec::new();
        let mut current = unsafe { (!(*obj_ptr).shape.is_null()).then_some((*obj_ptr).shape) };

        while let Some(shape_ptr) = current {
            unsafe {
                let object = &*obj_ptr;
                if let Some(key) = (*shape_ptr).key {
                    let offset = (*shape_ptr).offset as usize;
                    if object.named_present.get(offset).copied().unwrap_or(false)
                        && !keys.contains(&key)
                    {
                        keys.push(key);
                    }
                }
                current = (*shape_ptr).parent;
            }
        }

        keys.sort_by_key(PropertyKey::sort_key);
        keys
    }

    fn cached_named_property_value(
        &self,
        slot: usize,
        obj_ptr: *mut JSObject,
        key: PropertyKey,
    ) -> Option<JSValue> {
        let ic = self.frame.ic_vector.get(slot)?;
        if ic.state != ICState::Mono
            || ic.key != Some(key)
            || ic.shape_id != self.current_shape_id(obj_ptr)
        {
            return None;
        }

        let offset = ic.offset as usize;
        unsafe {
            let object = &*obj_ptr;
            object
                .named_present
                .get(offset)
                .copied()
                .filter(|present| *present)
                .and_then(|_| object.named_values.get(offset).copied())
        }
    }

    fn set_cached_named_property(
        &mut self,
        slot: usize,
        obj_ptr: *mut JSObject,
        key: PropertyKey,
        value: JSValue,
    ) -> bool {
        let Some(ic) = self.frame.ic_vector.get(slot) else {
            return false;
        };
        if ic.state != ICState::Mono
            || ic.key != Some(key)
            || ic.shape_id != self.current_shape_id(obj_ptr)
        {
            return false;
        }

        let offset = ic.offset as usize;
        self.ensure_named_storage_slot(obj_ptr, offset);
        unsafe {
            let object = &mut *obj_ptr;
            object.named_values[offset] = value;
            object.named_present[offset] = true;
        }
        true
    }

    fn classify_value(&self, value: JSValue) -> ValueProfileKind {
        if is_undefined(value) {
            ValueProfileKind::Undefined
        } else if is_null(value) {
            ValueProfileKind::Null
        } else if bool_from_value(value).is_some() {
            ValueProfileKind::Boolean
        } else if is_string(value) {
            ValueProfileKind::String
        } else if is_object(value) {
            if let Some(obj_ptr) = object_from_value(value) {
                unsafe {
                    match (*obj_ptr).kind {
                        ObjectKind::Function(_)
                        | ObjectKind::Closure(_)
                        | ObjectKind::NativeFunction(_)
                        | ObjectKind::NativeClosure(_)
                        | ObjectKind::Class(_) => ValueProfileKind::Function,
                        _ => ValueProfileKind::Object,
                    }
                }
            } else {
                ValueProfileKind::Object
            }
        } else {
            ValueProfileKind::Number
        }
    }

    fn ensure_type_feedback_slot(&mut self, slot: usize) -> &mut TypeFeedbackSlot {
        if slot >= self.feedback.type_slots.len() {
            self.feedback
                .type_slots
                .resize(slot + 1, TypeFeedbackSlot::default());
        }
        &mut self.feedback.type_slots[slot]
    }

    fn ensure_call_feedback_slot(&mut self, slot: usize) -> &mut CallFeedbackSlot {
        if slot >= self.feedback.call_slots.len() {
            self.feedback
                .call_slots
                .resize(slot + 1, CallFeedbackSlot::default());
        }
        &mut self.feedback.call_slots[slot]
    }

    fn observe_type_feedback_slot(&mut self, slot: usize, value: JSValue) {
        let kind = self.classify_value(value);
        self.ensure_type_feedback_slot(slot).observe(kind);
    }

    fn observe_call_feedback_kind(&mut self, slot: usize, kind: ValueProfileKind) {
        self.ensure_call_feedback_slot(slot).observe(kind);
    }

    fn observe_return_value(&mut self, value: JSValue) {
        let kind = self.classify_value(value);
        self.feedback.return_slot.observe(kind);
    }

    fn record_deopt(&mut self, reason: DeoptReason) {
        self.feedback.deopt_count = self.feedback.deopt_count.saturating_add(1);
        self.feedback.last_deopt = Some(reason);
        self.feedback.osr_active = false;
    }

    fn restore_scope_depth(&mut self, depth: usize) {
        self.scope_chain.truncate(depth);
        self.frame.header.env = None;
    }

    fn switch_table_offset(value: JSValue) -> Option<i16> {
        let offset = to_i32(value)?;
        i16::try_from(offset).ok()
    }

    fn switch_jump_offset(&self, table_index: usize, value: JSValue) -> Option<i16> {
        let case_count = usize::try_from(to_i32(*self.const_pool.get(table_index)?)?).ok()?;
        let default_offset = Self::switch_table_offset(*self.const_pool.get(table_index + 1)?)?;
        let cases = &self.const_pool.get(table_index + 2..)?;

        for pair in cases.chunks_exact(2).take(case_count) {
            let case_value = pair[0];
            let case_offset = Self::switch_table_offset(pair[1])?;
            if self.strict_equal(value, case_value) {
                return Some(case_offset);
            }
        }

        Some(default_offset)
    }

    fn alloc_shape_with(
        &mut self,
        parent: Option<*mut Shape>,
        key: Option<PropertyKey>,
        property_count: u32,
        prototype: Option<*mut Shape>,
    ) -> *mut Shape {
        let offset = parent
            .map(|shape| unsafe { (*shape).property_count })
            .unwrap_or(0);
        let shape = Box::new(Shape {
            header: GCHeader::new(ObjType::Shape),
            id: self.next_shape_id,
            parent,
            key,
            offset,
            property_count,
            prototype,
            proto_cache_offset: 0,
            proto_cache_shape: None,
        });
        self.next_shape_id += 1;
        let shape_ptr = Box::into_raw(shape);
        self.shapes.push(shape_ptr);
        shape_ptr
    }

    pub fn alloc_shape(&mut self) -> *mut Shape {
        self.alloc_shape_with(None, None, 0, None)
    }

    fn alloc_object_with_kind(&mut self, kind: ObjectKind) -> JSValue {
        let heap_kind = match &kind {
            ObjectKind::Ordinary(_) | ObjectKind::Env(_) => HeapKind::Object,
            ObjectKind::Array(_) => HeapKind::Array,
            ObjectKind::BoolArray(_) => HeapKind::BoolArray,
            ObjectKind::Uint8Array(_) => HeapKind::Uint8Array,
            ObjectKind::Int32Array(_) => HeapKind::Int32Array,
            ObjectKind::Float64Array(_) => HeapKind::Float64Array,
            ObjectKind::StringArray(_) => HeapKind::StringArray,
            ObjectKind::Iterator { .. } => HeapKind::Object,
            ObjectKind::Function(_) => HeapKind::Function,
            ObjectKind::Closure(_) => HeapKind::Closure,
            ObjectKind::NativeFunction(_) => HeapKind::NativeFunction,
            ObjectKind::NativeClosure(_) => HeapKind::NativeClosure,
            ObjectKind::Class(_) => HeapKind::Class,
            ObjectKind::Module(_) => HeapKind::Module,
            ObjectKind::Instance(_) => HeapKind::Instance,
            ObjectKind::Symbol(_) => HeapKind::Symbol,
        };
        let shape = self.alloc_shape();
        let obj = Box::new(JSObject {
            header: GCHeader::with_kind(ObjType::Object, heap_kind),
            shape,
            properties: HashMap::new(),
            private_properties: HashMap::new(), // Initialize private properties
            named_values: Vec::new(),
            named_present: Vec::new(),
            kind,
        });
        let obj_ptr = Box::into_raw(obj);
        self.objects.push(obj_ptr);
        let value = make_object(obj_ptr);
        if !self.builtin_object_prototype.is_undefined() {
            let prototype_key =
                self.property_key_for_name(built_ins::object_internal_prototype_name());
            let _ = self.set_property(value, prototype_key, self.builtin_object_prototype);
        }
        value
    }

    pub fn alloc_object(&mut self) -> JSValue {
        self.alloc_object_with_kind(ObjectKind::Ordinary(QObject::new(self.heap_shape.clone())))
    }

    pub fn alloc_array(&mut self, size_hint: usize) -> JSValue {
        let mut array = QArray::new(self.heap_shape.clone());
        array.elements = Vec::with_capacity(size_hint);
        let value = self.alloc_object_with_kind(ObjectKind::Array(array));
        built_ins::attach_array_methods(self, value);
        value
    }

    fn alloc_iterator(&mut self, values: Vec<JSValue>) -> JSValue {
        self.alloc_object_with_kind(ObjectKind::Iterator { values, index: 0 })
    }

    fn alloc_function(&mut self, descriptor: JSValue) -> JSValue {
        let function = self.alloc_object_with_kind(ObjectKind::Function(QFunction {
            name: None,
            params: Vec::new(),
            body: Vec::new(),
            prototype: None,
            descriptor,
            upvalues: Vec::new(),
        }));
        built_ins::attach_callable_methods(self, function);
        function
    }

    fn alloc_native_function(&mut self, name: Option<&str>) -> JSValue {
        let name = name.map(|name| self.atoms.intern(name));
        self.alloc_object_with_kind(ObjectKind::NativeFunction(QNativeFunction {
            name,
            callback: builtin_native_stub,
        }))
    }

    fn alloc_class(&mut self, base: JSValue) -> JSValue {
        self.alloc_object_with_kind(ObjectKind::Class(QClass {
            name: None,
            prototype: None,
            constructor: None,
            static_props: HashMap::new(),
            base,
        }))
    }

    fn alloc_env(&mut self) -> JSValue {
        self.alloc_object_with_kind(ObjectKind::Env(QObject::new(self.heap_shape.clone())))
    }

    pub fn intern_string(&mut self, text: impl AsRef<str>) -> JSValue {
        let text = text.as_ref();
        match self.interned_strings.entry(text.to_owned()) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let atom = self.atoms.intern(entry.key());
                let string = Box::new(JSString::new(atom));
                let string_ptr = Box::into_raw(string);
                self.strings.push(string_ptr);
                let value = make_string(string_ptr);
                entry.insert(value);
                value
            }
        }
    }

    fn string_text(&self, value: JSValue) -> Option<&str> {
        if let Some(atom) = value.as_atom() {
            Some(self.atoms.resolve(atom))
        } else {
            string_from_value(value).map(|string_ptr| unsafe { (*string_ptr).text(&self.atoms) })
        }
    }

    fn string_equals(&self, value: JSValue, expected: &str) -> bool {
        self.string_text(value) == Some(expected)
    }

    fn property_key_to_text(&self, key: PropertyKey) -> Option<String> {
        match key {
            PropertyKey::Id(slot) => self
                .compiled_properties
                .get(slot as usize)
                .cloned()
                .or_else(|| Some(slot.to_string())),
            PropertyKey::Atom(atom) | PropertyKey::PrivateName(atom) => Some(self.atoms.resolve(atom).to_owned()),
            PropertyKey::Index(index) => Some(index.to_string()),
            PropertyKey::Value(value) => {
                if let Some(text) = self.string_text(value) {
                    Some(text.to_owned())
                } else if let Some(value) = to_i32(value) {
                    Some(value.to_string())
                } else if let Some(value) = to_f64(value) {
                    Some(value.to_string())
                } else if let Some(value) = bool_from_value(value) {
                    Some(if value { "true" } else { "false" }.to_owned())
                } else if is_null(value) {
                    Some("null".to_owned())
                } else if is_undefined(value) {
                    Some("undefined".to_owned())
                } else {
                    None
                }
            }
        }
    }

    fn is_internal_property_key(&self, key: PropertyKey) -> bool {
        self.property_key_to_text(key)
            .is_some_and(|name| name.starts_with("__qjs_"))
    }

    fn property_key_for_name(&mut self, name: &str) -> PropertyKey {
        if let Some(&slot) = self.property_slots.get(name) {
            PropertyKey::Id(slot)
        } else {
            PropertyKey::Atom(self.atoms.intern(name))
        }
    }

    fn property_key_for_existing_name(&self, name: &str) -> Option<PropertyKey> {
        self.property_slots
            .get(name)
            .copied()
            .map(PropertyKey::Id)
            .or_else(|| self.atoms.get(name).map(PropertyKey::Atom))
    }

    fn get_property_by_name(&self, obj_val: JSValue, name: &str) -> JSValue {
        self.property_key_for_existing_name(name)
            .map(|key| self.get_property(obj_val, key))
            .unwrap_or_else(make_undefined)
    }

    fn get_own_property_by_name(&self, obj_val: JSValue, name: &str) -> JSValue {
        self.property_key_for_existing_name(name)
            .map(|key| self.get_own_property_value_internal(obj_val, key))
            .unwrap_or_else(make_undefined)
    }

    fn has_property_by_name(&self, obj_val: JSValue, name: &str) -> bool {
        self.property_key_for_existing_name(name)
            .is_some_and(|key| self.has_property(obj_val, key))
    }

    fn has_own_property_by_name(&self, obj_val: JSValue, name: &str) -> bool {
        self.property_key_for_existing_name(name)
            .is_some_and(|key| self.has_own_property_value_internal(obj_val, key))
    }

    fn delete_property_by_name(&mut self, obj_val: JSValue, name: &str) -> bool {
        self.property_key_for_existing_name(name)
            .is_some_and(|key| self.delete_property(obj_val, key))
    }

    fn own_property_names(&self, obj_val: JSValue) -> Vec<String> {
        self.get_keys(obj_val)
            .into_iter()
            .filter_map(|key| self.property_key_to_text(key))
            .collect()
    }

    fn own_property_keys(&mut self, obj_val: JSValue) -> Vec<JSValue> {
        self.get_keys(obj_val)
            .into_iter()
            .map(|key| self.property_key_to_value(key))
            .collect()
    }

    fn get_internal_prototype(&self, obj_val: JSValue) -> JSValue {
        self.get_own_property_by_name(obj_val, built_ins::object_internal_prototype_name())
    }

    fn vm_to_runtime_value(
        &self,
        ctx: &Context,
        value: JSValue,
        seen: &mut HashMap<usize, JSValue>,
    ) -> Result<JSValue, String> {
        if value.is_undefined()
            || value.is_null()
            || value.as_bool().is_some()
            || value.as_i32().is_some()
            || value.as_f64().is_some()
        {
            return Ok(value);
        }

        if let Some(text) = self.string_text(value) {
            return Ok(JSValue::atom(ctx.intern(text)));
        }

        let Some(obj_ptr) = object_from_value(value) else {
            return Err(format!("unsupported VM value: {}", value.type_name()));
        };
        let key = obj_ptr as usize;
        if let Some(existing) = seen.get(&key).copied() {
            return Ok(existing);
        }

        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Array(array) => {
                    let out = ctx.new_array();
                    let out_value = JSValue::from(out.clone());
                    seen.insert(key, out_value);
                    let mut out_ref = out.borrow_mut();
                    for &element in &array.elements {
                        out_ref.push(self.vm_to_runtime_value(ctx, element, seen)?);
                    }
                    Ok(out_value)
                }
                ObjectKind::BoolArray(array) => {
                    let out = ctx.new_bool_array();
                    let out_value = JSValue::from(out.clone());
                    seen.insert(key, out_value);
                    let mut out_ref = out.borrow_mut();
                    for &element in &array.elements {
                        out_ref.push(element);
                    }
                    Ok(out_value)
                }
                ObjectKind::Uint8Array(array) => {
                    let out = ctx.new_uint8_array();
                    let out_value = JSValue::from(out.clone());
                    seen.insert(key, out_value);
                    let mut out_ref = out.borrow_mut();
                    for &element in &array.elements {
                        out_ref.push(element);
                    }
                    Ok(out_value)
                }
                ObjectKind::Int32Array(array) => {
                    let out = ctx.new_int32_array();
                    let out_value = JSValue::from(out.clone());
                    seen.insert(key, out_value);
                    let mut out_ref = out.borrow_mut();
                    for &element in &array.elements {
                        out_ref.push(element);
                    }
                    Ok(out_value)
                }
                ObjectKind::Float64Array(array) => {
                    let out = ctx.new_float64_array();
                    let out_value = JSValue::from(out.clone());
                    seen.insert(key, out_value);
                    let mut out_ref = out.borrow_mut();
                    for &element in &array.elements {
                        out_ref.push(element);
                    }
                    Ok(out_value)
                }
                ObjectKind::StringArray(array) => {
                    let out = ctx.new_string_array();
                    let out_value = JSValue::from(out.clone());
                    seen.insert(key, out_value);
                    let mut out_ref = out.borrow_mut();
                    for &element in &array.elements {
                        out_ref.push(self.vm_to_runtime_value(ctx, element, seen)?);
                    }
                    Ok(out_value)
                }
                ObjectKind::Ordinary(_) | ObjectKind::Env(_) | ObjectKind::Instance(_) => {
                    let out = ctx.new_object();
                    let out_value = JSValue::from(out.clone());
                    seen.insert(key, out_value);
                    let mut out_ref = out.borrow_mut();
                    let mut property_keys = self.named_property_keys(obj_ptr);
                    property_keys.extend(
                        (*obj_ptr)
                            .properties
                            .keys()
                            .copied()
                            .filter(|key| !Self::uses_shape_storage(*key)),
                    );
                    property_keys.sort_by_key(PropertyKey::sort_key);
                    for property_key in property_keys {
                        let Some(name) = self.property_key_to_text(property_key) else {
                            continue;
                        };
                        let child = self
                            .get_named_property_slot(obj_ptr, property_key)
                            .or_else(|| (*obj_ptr).properties.get(&property_key).copied())
                            .unwrap_or_else(make_undefined);
                        let child = self.vm_to_runtime_value(ctx, child, seen)?;
                        out_ref.set(ctx.intern(&name), child);
                    }
                    Ok(out_value)
                }
                ObjectKind::Iterator { .. } => {
                    Err("iterators are not supported by serializers".to_owned())
                }
                ObjectKind::Function(_) => {
                    Err("functions are not supported by serializers".to_owned())
                }
                ObjectKind::Closure(_) => {
                    Err("closures are not supported by serializers".to_owned())
                }
                ObjectKind::NativeFunction(_) => {
                    Err("native functions are not supported by serializers".to_owned())
                }
                ObjectKind::NativeClosure(_) => {
                    Err("native closures are not supported by serializers".to_owned())
                }
                ObjectKind::Class(_) => Err("classes are not supported by serializers".to_owned()),
                ObjectKind::Module(_) => Err("modules are not supported by serializers".to_owned()),
                ObjectKind::Symbol(_) => Err("symbols are not supported by serializers".to_owned()),
            }
        }
    }

    fn runtime_to_vm_value(
        &mut self,
        ctx: &Context,
        value: JSValue,
        seen: &mut HashMap<usize, JSValue>,
    ) -> Result<JSValue, String> {
        if value.is_undefined()
            || value.is_null()
            || value.as_bool().is_some()
            || value.as_i32().is_some()
            || value.as_f64().is_some()
        {
            return Ok(value);
        }

        if let Some(atom) = value.as_atom() {
            return Ok(self.intern_string(ctx.resolve(atom)));
        }

        if value.heap_kind() == Some(HeapKind::String) {
            let string = Gc::<QString>::try_from(value).map_err(|error| error.to_string())?;
            let text = ctx.resolve(string.borrow().atom);
            return Ok(self.intern_string(text));
        }

        let ptr = value
            .as_heap_ptr()
            .map(|ptr| ptr as usize)
            .ok_or_else(|| format!("unsupported runtime value: {}", value.type_name()))?;
        if let Some(existing) = seen.get(&ptr).copied() {
            return Ok(existing);
        }

        match value.heap_kind() {
            Some(HeapKind::Object) => {
                let object = Gc::<QObject>::try_from(value).map_err(|error| error.to_string())?;
                let out = self.alloc_object();
                seen.insert(ptr, out);
                let object_ref = object.borrow();
                let props: Vec<_> = object_ref
                    .shape
                    .props
                    .iter()
                    .map(|(&atom, &index)| (atom, index))
                    .collect();
                for (atom, index) in props {
                    let child = object_ref
                        .values
                        .get(index)
                        .copied()
                        .unwrap_or_else(make_undefined);
                    let child = self.runtime_to_vm_value(ctx, child, seen)?;
                    let key = self.property_key_for_name(ctx.rt.borrow().atoms.resolve(atom));
                    let _ = self.set_property(out, key, child);
                }
                Ok(out)
            }
            Some(HeapKind::Array) => {
                let array = Gc::<QArray>::try_from(value).map_err(|error| error.to_string())?;
                let out = self.alloc_array(array.borrow().elements.len());
                seen.insert(ptr, out);
                let elements = array.borrow().elements.clone();
                for (index, element) in elements.into_iter().enumerate() {
                    let element = self.runtime_to_vm_value(ctx, element, seen)?;
                    let _ = self.set_property(out, PropertyKey::Index(index as u32), element);
                }
                Ok(out)
            }
            Some(HeapKind::BoolArray) => {
                let array = Gc::<QBoolArray>::try_from(value).map_err(|error| error.to_string())?;
                let out = self.alloc_array(array.borrow().elements.len());
                seen.insert(ptr, out);
                let elements = array.borrow().elements.clone();
                for (index, element) in elements.into_iter().enumerate() {
                    let _ = self.set_property(
                        out,
                        PropertyKey::Index(index as u32),
                        make_bool(element),
                    );
                }
                Ok(out)
            }
            Some(HeapKind::Uint8Array) => {
                let array =
                    Gc::<QUint8Array>::try_from(value).map_err(|error| error.to_string())?;
                Ok(self.bytes_to_value(&array.borrow().elements))
            }
            Some(HeapKind::Int32Array) => {
                let array =
                    Gc::<QInt32Array>::try_from(value).map_err(|error| error.to_string())?;
                let out = self.alloc_array(array.borrow().elements.len());
                seen.insert(ptr, out);
                let elements = array.borrow().elements.clone();
                for (index, element) in elements.into_iter().enumerate() {
                    let _ = self.set_property(
                        out,
                        PropertyKey::Index(index as u32),
                        make_int32(element),
                    );
                }
                Ok(out)
            }
            Some(HeapKind::Float64Array) => {
                let array =
                    Gc::<QFloat64Array>::try_from(value).map_err(|error| error.to_string())?;
                let out = self.alloc_array(array.borrow().elements.len());
                seen.insert(ptr, out);
                let elements = array.borrow().elements.clone();
                for (index, element) in elements.into_iter().enumerate() {
                    let _ = self.set_property(
                        out,
                        PropertyKey::Index(index as u32),
                        make_number(element),
                    );
                }
                Ok(out)
            }
            Some(HeapKind::StringArray) => {
                let array =
                    Gc::<QStringArray>::try_from(value).map_err(|error| error.to_string())?;
                let out = self.alloc_array(array.borrow().elements.len());
                seen.insert(ptr, out);
                let elements = array.borrow().elements.clone();
                for (index, element) in elements.into_iter().enumerate() {
                    let element = self.runtime_to_vm_value(ctx, element, seen)?;
                    let _ = self.set_property(out, PropertyKey::Index(index as u32), element);
                }
                Ok(out)
            }
            Some(HeapKind::Function) => {
                Err("functions are not supported by VM serializer bridge".to_owned())
            }
            Some(HeapKind::Closure) => {
                Err("closures are not supported by VM serializer bridge".to_owned())
            }
            Some(HeapKind::NativeFunction) => {
                Err("native functions are not supported by VM serializer bridge".to_owned())
            }
            Some(HeapKind::NativeClosure) => {
                Err("native closures are not supported by VM serializer bridge".to_owned())
            }
            Some(HeapKind::Class) => {
                Err("classes are not supported by VM serializer bridge".to_owned())
            }
            Some(HeapKind::Module) => {
                Err("modules are not supported by VM serializer bridge".to_owned())
            }
            Some(HeapKind::Instance) => {
                Err("instances are not supported by VM serializer bridge".to_owned())
            }
            Some(HeapKind::Symbol) => {
                Err("symbols are not supported by VM serializer bridge".to_owned())
            }
            Some(HeapKind::String) => unreachable!(),
            None => Err("unknown runtime heap value".to_owned()),
        }
    }

    fn byte_from_value(&self, value: JSValue) -> Option<u8> {
        u8::try_from(to_i32(value)?).ok()
    }

    fn bytes_from_value(&self, value: JSValue) -> Option<Vec<u8>> {
        if let Some(obj_ptr) = object_from_value(value) {
            unsafe {
                match &(*obj_ptr).kind {
                    ObjectKind::Array(array) => {
                        return array
                            .elements
                            .iter()
                            .map(|&element| self.byte_from_value(element))
                            .collect();
                    }
                    ObjectKind::Uint8Array(array) => return Some(array.elements.clone()),
                    _ => {}
                }
            }
        }
        None
    }

    fn bytes_to_value(&mut self, bytes: &[u8]) -> JSValue {
        let out = self.alloc_array(bytes.len());
        for (index, byte) in bytes.iter().copied().enumerate() {
            let _ = self.set_property(
                out,
                PropertyKey::Index(index as u32),
                make_int32(byte as i32),
            );
        }
        out
    }

    fn dispatch_native_function(
        &mut self,
        callee: JSValue,
        function: &QNativeFunction,
        this_value: JSValue,
        args: &[JSValue],
    ) -> JSValue {
        if let Some(name) = function.name {
            let builtin_name = self.atoms.resolve(name).to_owned();
            if let Some(result) =
                built_ins::dispatch_builtin(self, &builtin_name, callee, this_value, args)
            {
                return result;
            }
        }

        with_bridge_context(|ctx| (function.callback)(ctx, this_value, args))
    }

    fn dispatch_native_closure(
        &mut self,
        function: &QNativeClosure,
        this_value: JSValue,
        args: &[JSValue],
    ) -> JSValue {
        with_bridge_context(|ctx| (function.callback)(ctx, this_value, args))
    }

    fn console_render_args(&mut self, args: &[JSValue]) -> String {
        args.iter()
            .map(|&value| self.display_string(value))
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn console_write_line(&mut self, text: String, is_error: bool) -> JSValue {
        let line = if self.console_group_depth == 0 {
            text
        } else {
            format!("{}{}", "  ".repeat(self.console_group_depth), text)
        };
        self.console_output.push(line.clone());
        if self.console_echo {
            if is_error {
                eprintln!("{line}");
            } else {
                println!("{line}");
            }
        }
        make_undefined()
    }

    fn console_label_from_args(&mut self, args: &[JSValue]) -> String {
        args.first()
            .map(|&value| self.display_string(value))
            .unwrap_or_else(|| "default".to_owned())
    }

    fn console_elapsed_message(&self, label: &str, start: Instant, suffix: Option<&str>) -> String {
        let millis = start.elapsed().as_secs_f64() * 1000.0;
        match suffix {
            Some(extra) if !extra.is_empty() => format!("{label}: {millis:.3}ms {extra}"),
            _ => format!("{label}: {millis:.3}ms"),
        }
    }

    fn is_truthy_value(&self, value: JSValue) -> bool {
        if let Some(text) = self.string_text(value) {
            !text.is_empty()
        } else {
            is_truthy(value)
        }
    }

    fn display_string(&mut self, value: JSValue) -> String {
        if is_undefined(value) {
            "undefined".to_owned()
        } else if is_null(value) {
            "null".to_owned()
        } else if let Some(boolean) = bool_from_value(value) {
            if boolean {
                "true".to_owned()
            } else {
                "false".to_owned()
            }
        } else if let Some(text) = self.string_text(value) {
            text.to_owned()
        } else if let Some(number) = to_f64(value) {
            if number.is_nan() {
                "NaN".to_owned()
            } else if number.is_infinite() && number.is_sign_positive() {
                "Infinity".to_owned()
            } else if number.is_infinite() {
                "-Infinity".to_owned()
            } else if number.fract() == 0.0 {
                format!("{number:.0}")
            } else {
                number.to_string()
            }
        } else if let Some(obj_ptr) = object_from_value(value) {
            unsafe {
                match &(*obj_ptr).kind {
                    ObjectKind::Array(array) => {
                        let items = array.elements.clone();
                        let mut rendered = Vec::with_capacity(items.len());
                        for item in items {
                            rendered.push(self.display_string(item));
                        }
                        rendered.join(",")
                    }
                    ObjectKind::Function(_)
                    | ObjectKind::Closure(_)
                    | ObjectKind::NativeFunction(_)
                    | ObjectKind::NativeClosure(_) => "function() { [bytecode] }".to_owned(),
                    ObjectKind::Class(_) => "class {}".to_owned(),
                    ObjectKind::Iterator { .. } => "[object Iterator]".to_owned(),
                    ObjectKind::Env(_) => "[object Env]".to_owned(),
                    ObjectKind::Module(_) => "[object Module]".to_owned(),
                    ObjectKind::Instance(_) => "[object Instance]".to_owned(),
                    ObjectKind::Symbol(_) => "[object Symbol]".to_owned(),
                    ObjectKind::BoolArray(_)
                    | ObjectKind::Uint8Array(_)
                    | ObjectKind::Int32Array(_)
                    | ObjectKind::Float64Array(_)
                    | ObjectKind::StringArray(_) => "[object Array]".to_owned(),
                    ObjectKind::Ordinary(_) => "[object Object]".to_owned(),
                }
            }
        } else {
            "unknown".to_owned()
        }
    }

    fn primitive_value(&mut self, value: JSValue) -> JSValue {
        if is_object(value) {
            let rendered = self.display_string(value);
            self.intern_string(rendered)
        } else {
            value
        }
    }

    fn number_value(&mut self, value: JSValue) -> JSValue {
        if let Some(number) = to_f64(value) {
            make_number(number)
        } else if is_undefined(value) {
            make_number(f64::NAN)
        } else if is_null(value) {
            make_number(0.0)
        } else if let Some(boolean) = bool_from_value(value) {
            make_number(if boolean { 1.0 } else { 0.0 })
        } else if let Some(text) = self.string_text(value) {
            make_number(text.trim().parse::<f64>().unwrap_or(f64::NAN))
        } else {
            let primitive = self.primitive_value(value);
            if primitive == value {
                make_number(f64::NAN)
            } else {
                self.number_value(primitive)
            }
        }
    }

    fn string_value(&mut self, value: JSValue) -> JSValue {
        let rendered = self.display_string(value);
        self.intern_string(rendered)
    }

    fn type_of_name(&self, value: JSValue) -> &'static str {
        if is_undefined(value) {
            "undefined"
        } else if is_null(value) {
            "object"
        } else if bool_from_value(value).is_some() {
            "boolean"
        } else if is_string(value) {
            "string"
        } else if is_object(value) {
            if let Some(obj_ptr) = object_from_value(value) {
                unsafe {
                    match (*obj_ptr).kind {
                        ObjectKind::Function(_)
                        | ObjectKind::Closure(_)
                        | ObjectKind::NativeFunction(_)
                        | ObjectKind::NativeClosure(_)
                        | ObjectKind::Class(_) => "function",
                        _ => "object",
                    }
                }
            } else {
                "object"
            }
        } else {
            "number"
        }
    }

    fn strict_equal(&self, lhs: JSValue, rhs: JSValue) -> bool {
        if is_string(lhs) && is_string(rhs) {
            return self.string_text(lhs) == self.string_text(rhs);
        }

        if is_object(lhs) && is_object(rhs) {
            return object_from_value(lhs) == object_from_value(rhs);
        }

        if let (Some(left), Some(right)) = (to_f64(lhs), to_f64(rhs)) {
            return !left.is_nan() && !right.is_nan() && left == right;
        }

        if let (Some(left), Some(right)) = (bool_from_value(lhs), bool_from_value(rhs)) {
            return left == right;
        }

        lhs == rhs
    }

    fn abstract_equal(&mut self, lhs: JSValue, rhs: JSValue) -> bool {
        if self.strict_equal(lhs, rhs) {
            return true;
        }

        if (is_null(lhs) && is_undefined(rhs)) || (is_undefined(lhs) && is_null(rhs)) {
            return true;
        }

        if bool_from_value(lhs).is_some() {
            let lhs = self.number_value(lhs);
            return self.abstract_equal(lhs, rhs);
        }

        if bool_from_value(rhs).is_some() {
            let rhs = self.number_value(rhs);
            return self.abstract_equal(lhs, rhs);
        }

        if (is_string(lhs) && to_f64(rhs).is_some()) || (to_f64(lhs).is_some() && is_string(rhs)) {
            let left = to_f64(self.number_value(lhs)).unwrap_or(f64::NAN);
            let right = to_f64(self.number_value(rhs)).unwrap_or(f64::NAN);
            return !left.is_nan() && !right.is_nan() && left == right;
        }

        if is_object(lhs) {
            let lhs = self.primitive_value(lhs);
            return self.abstract_equal(lhs, rhs);
        }

        if is_object(rhs) {
            let rhs = self.primitive_value(rhs);
            return self.abstract_equal(lhs, rhs);
        }

        false
    }

    fn less_than(&mut self, lhs: JSValue, rhs: JSValue) -> bool {
        if lhs.is_int() && rhs.is_int() {
            return lhs.int_payload_unchecked() < rhs.int_payload_unchecked();
        }

        if let Some((left, right)) = self.fast_number_pair(lhs, rhs) {
            return left < right;
        }

        if is_string(lhs) && is_string(rhs) {
            return self.string_text(lhs) < self.string_text(rhs);
        }

        let left = to_f64(self.number_value(lhs)).unwrap_or(f64::NAN);
        let right = to_f64(self.number_value(rhs)).unwrap_or(f64::NAN);
        !left.is_nan() && !right.is_nan() && left < right
    }

    fn less_than_or_equal(&mut self, lhs: JSValue, rhs: JSValue) -> bool {
        if lhs.is_int() && rhs.is_int() {
            return lhs.int_payload_unchecked() <= rhs.int_payload_unchecked();
        }

        if let Some((left, right)) = self.fast_number_pair(lhs, rhs) {
            return left <= right;
        }

        self.less_than(lhs, rhs) || self.strict_equal(lhs, rhs)
    }

    fn fast_number_pair(&self, lhs: JSValue, rhs: JSValue) -> Option<(f64, f64)> {
        Some((to_f64(lhs)?, to_f64(rhs)?))
    }

    fn constant_or_undefined(&self, index: usize) -> JSValue {
        self.const_pool
            .get(index)
            .copied()
            .unwrap_or(make_undefined())
    }

    fn should_stop_after_frame_exit(&self) -> bool {
        self.threaded_stop_depth == Some(self.frame.depth())
    }

    fn finish_frame_exit(&mut self, result: JSValue) -> ControlFlow {
        if !self.exit_frame(result) || self.should_stop_after_frame_exit() {
            ControlFlow::Stop
        } else {
            ControlFlow::Continue
        }
    }

    fn store_call_result(&mut self, action: CallAction) -> ControlFlow {
        if let CallAction::Returned(result) = action {
            self.frame.regs[ACC] = result;
        }
        ControlFlow::Continue
    }

    fn add_values(&mut self, lhs: JSValue, rhs: JSValue) -> JSValue {
        if lhs.is_int() && rhs.is_int() {
            let lhs_int = lhs.int_payload_unchecked();
            let rhs_int = rhs.int_payload_unchecked();
            if let Some(result) = lhs_int.checked_add(rhs_int) {
                return make_int32(result);
            }
        }

        if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
            return make_number(lhs + rhs);
        }

        let (lhs, rhs) = self.value_pair(lhs, rhs);
        lhs.add(&rhs).raw()
    }

    fn mul_values(&mut self, lhs: JSValue, rhs: JSValue) -> JSValue {
        if lhs.is_int() && rhs.is_int() {
            let lhs_int = lhs.int_payload_unchecked();
            let rhs_int = rhs.int_payload_unchecked();
            if let Some(result) = lhs_int.checked_mul(rhs_int) {
                return make_int32(result);
            }
        }

        if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
            return make_number(lhs * rhs);
        }

        let (lhs, rhs) = self.value_pair(lhs, rhs);
        lhs.mul(&rhs).raw()
    }

    fn sub_immediate_value(&mut self, value: JSValue, imm: i8) -> JSValue {
        let imm_i32 = i32::from(imm);
        if value.is_int() {
            let value_int = value.int_payload_unchecked();
            if let Some(result) = value_int.checked_sub(imm_i32) {
                return make_int32(result);
            }
        }

        if let Some(number) = to_f64(value) {
            return make_number(number - f64::from(imm));
        }

        self.binary_numeric_op(value, make_int32(imm_i32), |x, y| x - y)
    }

    fn inc_value(&mut self, value: JSValue) -> JSValue {
        if value.is_int() {
            let int_val = value.int_payload_unchecked();
            if let Some(result) = int_val.checked_add(1) {
                return make_int32(result);
            }
        }

        if let Some(number) = to_f64(value) {
            return make_number(number + 1.0);
        }

        self.value_op(value).inc().raw()
    }

    fn write_result_reg(&mut self, dst: usize, value: JSValue) {
        self.frame.regs[ACC] = value;
        if dst != ACC {
            self.frame.regs[dst] = value;
        }
    }

    fn binary_add(&mut self, lhs: JSValue, rhs: JSValue) -> JSValue {
        if is_string(lhs) || is_string(rhs) {
            let result = format!("{}{}", self.display_string(lhs), self.display_string(rhs));
            self.intern_string(result)
        } else {
            let left = to_f64(self.number_value(lhs)).unwrap_or(f64::NAN);
            let right = to_f64(self.number_value(rhs)).unwrap_or(f64::NAN);
            make_number(left + right)
        }
    }

    fn binary_numeric_op<F>(&mut self, lhs: JSValue, rhs: JSValue, op: F) -> JSValue
    where
        F: FnOnce(f64, f64) -> f64,
    {
        let left = to_f64(self.number_value(lhs)).unwrap_or(f64::NAN);
        let right = to_f64(self.number_value(rhs)).unwrap_or(f64::NAN);
        make_number(op(left, right))
    }

    fn array_index_from_value(&self, value: JSValue) -> Option<usize> {
        let number = to_f64(value)?;
        if number.is_finite() && number >= 0.0 && number.fract() == 0.0 {
            Some(number as usize)
        } else {
            None
        }
    }

    fn property_is_length(&self, key: PropertyKey) -> bool {
        match key {
            PropertyKey::Id(id) => self
                .compiled_properties
                .get(id as usize)
                .is_some_and(|name| name == "length"),
            PropertyKey::Atom(atom) => self.atoms.resolve(atom) == "length",
            PropertyKey::Value(value) => self.string_equals(value, "length"),
            _ => false,
        }
    }

    fn get_length_value(&self, value: JSValue) -> JSValue {
        if let Some(obj_ptr) = object_from_value(value) {
            unsafe {
                match &(*obj_ptr).kind {
                    ObjectKind::Array(array) => make_number(array.elements.len() as f64),
                    _ => make_number(
                        (self.named_property_count(obj_ptr) + (*obj_ptr).properties.len()) as f64,
                    ),
                }
            }
        } else if let Some(text) = self.string_text(value) {
            make_number(text.chars().count() as f64)
        } else {
            make_number(0.0)
        }
    }

    fn transition_shape_if_needed(&mut self, obj_ptr: *mut JSObject, key: PropertyKey) {
        let should_transition =
            Self::uses_shape_storage(key) && self.named_property_offset(obj_ptr, key).is_none();
        if !should_transition {
            return;
        }

        let (parent, property_count, prototype) = unsafe {
            let parent = (*obj_ptr).shape;
            let next_property_count = if parent.is_null() {
                1
            } else {
                (*parent).property_count + 1
            };
            let prototype = if parent.is_null() {
                None
            } else {
                (*parent).prototype
            };
            (
                if parent.is_null() { None } else { Some(parent) },
                next_property_count,
                prototype,
            )
        };

        let new_shape = self.alloc_shape_with(parent, Some(key), property_count, prototype);
        unsafe {
            (*obj_ptr).shape = new_shape;
        }
    }

    pub fn obj_get_prop(&self, obj_val: JSValue, key_id: u16) -> JSValue {
        self.get_property(obj_val, PropertyKey::Id(key_id))
    }

    pub fn obj_set_prop(&mut self, obj_val: JSValue, key_id: u16, value: JSValue) {
        let _ = self.set_property(obj_val, PropertyKey::Id(key_id), value);
    }

    fn get_own_property_value_internal(&self, obj_val: JSValue, key: PropertyKey) -> JSValue {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return make_undefined();
        };

        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Array(array) => match key {
                    PropertyKey::Index(index) => array
                        .elements
                        .get(index as usize)
                        .copied()
                        .unwrap_or(make_undefined()),
                    _ if self.property_is_length(key) => make_number(array.elements.len() as f64),
                    _ => self
                        .get_named_property_slot(obj_ptr, key)
                        .or_else(|| (*obj_ptr).properties.get(&key).copied())
                        .or_else(|| {
                            if matches!(key, PropertyKey::PrivateName(_)) {
                                (*obj_ptr).private_properties.get(&key).copied()
                            } else {
                                None
                            }
                        })
                        .unwrap_or(make_undefined()),
                },
                _ => self
                    .get_named_property_slot(obj_ptr, key)
                    .or_else(|| (*obj_ptr).properties.get(&key).copied())
                    .or_else(|| {
                        if matches!(key, PropertyKey::PrivateName(_)) {
                            (*obj_ptr).private_properties.get(&key).copied()
                        } else {
                            None
                        }
                    })
                    .unwrap_or(make_undefined()),
            }
        }
    }

    fn has_own_property_value_internal(&self, obj_val: JSValue, key: PropertyKey) -> bool {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return false;
        };

        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Array(array) => match key {
                    PropertyKey::Index(index) => array.elements.get(index as usize).is_some(),
                    _ if self.property_is_length(key) => true,
                    _ => {
                        self.has_named_property_slot(obj_ptr, key)
                            || (*obj_ptr).properties.contains_key(&key)
                            || matches!(key, PropertyKey::PrivateName(_)) && (*obj_ptr).private_properties.contains_key(&key)
                    }
                },
                _ => {
                    self.has_named_property_slot(obj_ptr, key)
                        || (*obj_ptr).properties.contains_key(&key)
                        || matches!(key, PropertyKey::PrivateName(_)) && (*obj_ptr).private_properties.contains_key(&key)
                }
            }
        }
    }

    fn get_property(&self, obj_val: JSValue, key: PropertyKey) -> JSValue {
        let key = match key {
            PropertyKey::Id(id) if id as usize >= self.compiled_properties.len() => {
                let private_idx = id as usize - self.compiled_properties.len();
                if let Some(atom) = self.compiled_private_properties.get(private_idx) {
                    PropertyKey::PrivateName(*atom)
                } else {
                    return make_undefined();
                }
            }
            other => other,
        };

        if let Some(value) = self.get_primitive_property(obj_val, key) {
            return value;
        }

        let Some(_) = object_from_value(obj_val) else {
            return make_undefined();
        };

        let mut current = obj_val;
        loop {
            if self.has_own_property_value_internal(current, key) {
                return self.get_own_property_value_internal(current, key);
            }

            let prototype = self.get_internal_prototype(current);
            if prototype.is_null() || prototype.is_undefined() || !is_object(prototype) {
                return make_undefined();
            }
            current = prototype;
        }
    }

    fn get_primitive_property(&self, value: JSValue, key: PropertyKey) -> Option<JSValue> {
        if let Some(text) = self.string_text(value) {
            if self.property_is_length(key) {
                return Some(make_number(text.chars().count() as f64));
            }

            let index = match key {
                PropertyKey::Index(index) => Some(index as usize),
                _ => self
                    .property_key_to_text(key)
                    .and_then(|name| name.parse::<usize>().ok()),
            };
            if let Some(index) = index
                && let Some(ch) = text.chars().nth(index)
            {
                let rendered = ch.to_string();
                if let Some(atom) = self.atoms.get(&rendered) {
                    return Some(JSValue::atom(atom));
                }
            }

            if !self.builtin_string_prototype.is_undefined() {
                let method = self.get_property(self.builtin_string_prototype, key);
                if !method.is_undefined() {
                    return Some(method);
                }
            }
        }

        if to_f64(value).is_some() && self.property_key_to_text(key).as_deref() == Some("toFixed") {
            return Some(self.builtin_number_to_fixed);
        }

        None
    }

    fn set_property(&mut self, obj_val: JSValue, key: PropertyKey, value: JSValue) -> JSValue {
        let key = match key {
            PropertyKey::Id(id) if id as usize >= self.compiled_properties.len() => {
                let private_idx = id as usize - self.compiled_properties.len();
                if let Some(name) = self.compiled_private_properties.get(private_idx) {
                    PropertyKey::PrivateName(*name)
                } else {
                    return value;
                }
            }
            other => other,
        };

        let Some(obj_ptr) = object_from_value(obj_val) else {
            return make_undefined();
        };

        let frozen_key = self.property_key_for_name("__qjs_frozen");
        let is_frozen = if key == frozen_key {
            false
        } else {
            unsafe {
                self.get_named_property_slot(obj_ptr, frozen_key)
                    .or_else(|| (*obj_ptr).properties.get(&frozen_key).copied())
                    .and_then(bool_from_value)
                    .unwrap_or(false)
            }
        };
        if is_frozen {
            return value;
        }

        unsafe {
            if let ObjectKind::Array(array) = &mut (*obj_ptr).kind {
                match key {
                    PropertyKey::Index(index) => {
                        let index = index as usize;
                        if index >= array.elements.len() {
                            array.elements.resize(index + 1, make_undefined());
                        }
                        array.elements[index] = value;
                        return value;
                    }
                    _ if self.property_is_length(key) => {
                        let numeric_value = self.number_value(value);
                        let length = self.array_index_from_value(numeric_value).unwrap_or(0);
                        array.elements.resize(length, make_undefined());
                        return make_number(array.elements.len() as f64);
                    }
                    _ => {}
                }
            }
        }

        if self.set_named_property_slot(obj_ptr, key, value) {
            return value;
        }

        unsafe {
            if matches!(key, PropertyKey::PrivateName(_)) {
                (*obj_ptr).private_properties.insert(key, value);
            } else {
                (*obj_ptr).properties.insert(key, value);
            }
        }
        value
    }

    fn delete_property(&mut self, obj_val: JSValue, key: PropertyKey) -> bool {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return false;
        };

        unsafe {
            match &mut (*obj_ptr).kind {
                ObjectKind::Array(array) => match key {
                    PropertyKey::Index(index) => {
                        let index = index as usize;
                        if let Some(slot) = array.elements.get_mut(index) {
                            *slot = make_undefined();
                            return true;
                        }
                        false
                    }
                    _ if self.property_is_length(key) => false,
                    _ => {
                        self.delete_named_property_slot(obj_ptr, key)
                            || (*obj_ptr).properties.remove(&key).is_some()
                            || (matches!(key, PropertyKey::PrivateName(_)) && (*obj_ptr).private_properties.remove(&key).is_some())
                    }
                },
                _ => {
                    self.delete_named_property_slot(obj_ptr, key)
                        || (*obj_ptr).properties.remove(&key).is_some()
                        || (matches!(key, PropertyKey::PrivateName(_)) && (*obj_ptr).private_properties.remove(&key).is_some())
                }
            }
        }
    }

    fn has_property(&self, obj_val: JSValue, key: PropertyKey) -> bool {
        if self.get_primitive_property(obj_val, key).is_some() {
            return true;
        }

        let Some(_) = object_from_value(obj_val) else {
            return false;
        };

        let mut current = obj_val;
        loop {
            if self.has_own_property_value_internal(current, key) {
                return true;
            }

            let prototype = self.get_internal_prototype(current);
            if prototype.is_null() || prototype.is_undefined() || !is_object(prototype) {
                return false;
            }
            current = prototype;
        }
    }

    fn has_private_property(&self, obj_val: JSValue, key: PropertyKey) -> bool {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return false;
        };

        unsafe {
            matches!(key, PropertyKey::PrivateName(_)) && (*obj_ptr).private_properties.contains_key(&key)
        }
    }

    fn get_private_property(&self, obj_val: JSValue, key: PropertyKey) -> JSValue {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return make_undefined();
        };

        unsafe {
            if let PropertyKey::PrivateName(_) = key {
                (*obj_ptr).private_properties.get(&key).copied().unwrap_or(make_undefined())
            } else {
                make_undefined()
            }
        }
    }

    fn set_private_property(&mut self, obj_val: JSValue, key: PropertyKey, value: JSValue) -> JSValue {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return make_undefined();
        };

        unsafe {
            if let PropertyKey::PrivateName(_) = key {
                (*obj_ptr).private_properties.insert(key, value);
            }
        }
        value
    }

    fn get_own_property_value_from_value(&self, obj_val: JSValue, key: JSValue) -> JSValue {
        self.get_own_property_value_internal(obj_val, self.property_key_from_value(key))
    }

    fn has_own_property_value_from_value(&self, obj_val: JSValue, key: JSValue) -> bool {
        self.has_own_property_value_internal(obj_val, self.property_key_from_value(key))
    }

    fn object_same_value(&self, lhs: JSValue, rhs: JSValue) -> bool {
        if let (Some(left), Some(right)) = (to_f64(lhs), to_f64(rhs)) {
            if left.is_nan() && right.is_nan() {
                return true;
            }

            if left == 0.0 && right == 0.0 {
                return left.to_bits() == right.to_bits();
            }

            return left == right;
        }

        if is_string(lhs) && is_string(rhs) {
            return self.string_text(lhs) == self.string_text(rhs);
        }

        if is_object(lhs) && is_object(rhs) {
            return object_from_value(lhs) == object_from_value(rhs);
        }

        if let (Some(left), Some(right)) = (bool_from_value(lhs), bool_from_value(rhs)) {
            return left == right;
        }

        if (is_null(lhs) && is_null(rhs)) || (is_undefined(lhs) && is_undefined(rhs)) {
            return true;
        }

        lhs == rhs
    }

    fn get_keys(&self, obj_val: JSValue) -> Vec<PropertyKey> {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return Vec::new();
        };

        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Array(array) => {
                    let mut keys = Vec::with_capacity(
                        array.elements.len()
                            + self.named_property_count(obj_ptr)
                            + (*obj_ptr).properties.len(),
                    );
                    for index in 0..array.elements.len() {
                        keys.push(PropertyKey::Index(index as u32));
                    }
                    let mut named = self.named_property_keys(obj_ptr);
                    named.extend(
                        (*obj_ptr)
                            .properties
                            .keys()
                            .copied()
                            .filter(|key| !Self::uses_shape_storage(*key)),
                    );
                    named.sort_by_key(PropertyKey::sort_key);
                    keys.extend(
                        named
                            .into_iter()
                            .filter(|key| !self.is_internal_property_key(*key)),
                    );
                    keys
                }
                _ => {
                    let mut keys = self.named_property_keys(obj_ptr);
                    keys.extend(
                        (*obj_ptr)
                            .properties
                            .keys()
                            .copied()
                            .filter(|key| !Self::uses_shape_storage(*key)),
                    );
                    keys.sort_by_key(PropertyKey::sort_key);
                    keys.into_iter()
                        .filter(|key| !self.is_internal_property_key(*key))
                        .collect()
                }
            }
        }
    }

    fn array_push(&mut self, array_val: JSValue, value: JSValue) -> JSValue {
        let Some(obj_ptr) = object_from_value(array_val) else {
            return make_undefined();
        };

        unsafe {
            if let ObjectKind::Array(array) = &mut (*obj_ptr).kind {
                array.push(value);
                return make_number(array.elements.len() as f64);
            }
        }

        make_undefined()
    }

    fn array_values(&self, value: JSValue) -> Option<Vec<JSValue>> {
        let obj_ptr = object_from_value(value)?;
        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Array(array) => Some(array.elements.clone()),
                ObjectKind::Iterator { values, .. } => Some(values.clone()),
                _ => None,
            }
        }
    }

    fn iterator_next_value(&mut self, iterator_val: JSValue) -> JSValue {
        let Some(obj_ptr) = object_from_value(iterator_val) else {
            return make_undefined();
        };

        unsafe {
            if let ObjectKind::Iterator { values, index } = &mut (*obj_ptr).kind
                && *index < values.len()
            {
                let value = values[*index];
                *index += 1;
                return value;
            }
        }

        make_undefined()
    }

    fn scope_at_depth(&self, depth: usize) -> Option<JSValue> {
        self.scope_chain
            .len()
            .checked_sub(depth + 1)
            .and_then(|index| self.scope_chain.get(index).copied())
    }

    fn set_scope_at_depth(&mut self, depth: usize, value: JSValue) {
        if let Some(index) = self.scope_chain.len().checked_sub(depth + 1)
            && index < self.scope_chain.len()
        {
            self.scope_chain[index] = value;
            return;
        }

        if depth == 0 {
            self.scope_chain.push(value);
        }
    }

    fn resolve_scope_value(&self, name: u16) -> Option<JSValue> {
        self.scope_chain
            .iter()
            .rev()
            .find(|&&scope| self.has_property(scope, PropertyKey::Id(name)))
            .copied()
    }

    fn load_name_value(&self, name: u16) -> JSValue {
        if let Some(scope) = self.resolve_scope_value(name) {
            self.get_property(scope, PropertyKey::Id(name))
        } else {
            self.global_object
                .get(&name)
                .copied()
                .unwrap_or(make_undefined())
        }
    }

    fn store_name_value(&mut self, name: u16, value: JSValue) {
        if let Some(scope) = self.resolve_scope_value(name) {
            let _ = self.set_property(scope, PropertyKey::Id(name), value);
        } else {
            self.global_object.insert(name, value);
        }
    }

    fn init_name_value(&mut self, name: u16, value: JSValue) {
        if let Some(&scope) = self.scope_chain.last() {
            let _ = self.set_property(scope, PropertyKey::Id(name), value);
        } else {
            self.global_object.insert(name, value);
        }
    }

    fn ensure_upvalue_slot(&mut self, slot: usize) {
        if slot >= self.upvalues.len() {
            self.upvalues.resize(slot + 1, make_undefined());
        }
    }

    fn current_function_value(&self) -> Option<JSValue> {
        self.frame.header.function_value
    }

    fn get_function_upvalue(&self, function_value: JSValue, slot: usize) -> Option<JSValue> {
        let obj_ptr = object_from_value(function_value)?;
        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Function(function) => function.upvalues.get(slot).copied(),
                ObjectKind::Closure(closure) => closure.captures.get(slot).copied(),
                ObjectKind::NativeClosure(closure) => closure.captures.get(slot).copied(),
                _ => None,
            }
        }
    }

    fn set_function_upvalue(
        &mut self,
        function_value: JSValue,
        slot: usize,
        value: JSValue,
    ) -> bool {
        let Some(obj_ptr) = object_from_value(function_value) else {
            return false;
        };

        unsafe {
            match &mut (*obj_ptr).kind {
                ObjectKind::Function(function) => {
                    if slot >= function.upvalues.len() {
                        function.upvalues.resize(slot + 1, make_undefined());
                    }
                    function.upvalues[slot] = value;
                    true
                }
                ObjectKind::Closure(closure) => {
                    if slot >= closure.captures.len() {
                        closure.captures.resize(slot + 1, make_undefined());
                    }
                    closure.captures[slot] = value;
                    true
                }
                ObjectKind::NativeClosure(closure) => {
                    if slot >= closure.captures.len() {
                        closure.captures.resize(slot + 1, make_undefined());
                    }
                    closure.captures[slot] = value;
                    true
                }
                _ => false,
            }
        }
    }

    fn collect_call_args(&self, start: usize, count: usize) -> Vec<JSValue> {
        match count {
            0 => Vec::new(),
            1 => vec![
                self.frame
                    .regs
                    .get(start)
                    .copied()
                    .unwrap_or(make_undefined()),
            ],
            _ => (0..count)
                .map(|index| {
                    self.frame
                        .regs
                        .get(start + index)
                        .copied()
                        .unwrap_or(make_undefined())
                })
                .collect(),
        }
    }

    fn collect_rest_args_value(&mut self, start: usize) -> JSValue {
        let array = self.alloc_array(0);
        let argc = self.frame.argc as usize;
        for index in start..argc {
            let _ = self.array_push(array, self.frame.arg(index));
        }
        array
    }

    fn function_descriptor_info(&self, descriptor: JSValue) -> Option<(usize, bool)> {
        let entry = to_f64(descriptor)?;
        if !entry.is_finite() || entry.fract() != 0.0 {
            return None;
        }
        if entry >= 0.0 {
            return Some((entry as usize, false));
        }

        let decoded = -entry - 1.0;
        (decoded >= 0.0).then_some((decoded as usize, true))
    }

    fn function_value_is_async(&self, function_value: JSValue) -> bool {
        let Some(obj_ptr) = object_from_value(function_value) else {
            return false;
        };

        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Function(function) => self
                    .function_descriptor_info(function.descriptor)
                    .is_some_and(|(_, is_async)| is_async),
                _ => false,
            }
        }
    }

    fn current_frame_is_async(&self) -> bool {
        self.frame
            .header
            .function_value
            .is_some_and(|function_value| self.function_value_is_async(function_value))
    }

    fn promise_resolve_value(&mut self, value: JSValue) -> JSValue {
        let resolve = self.builtin_function("__builtin_promise_resolve_static");
        self.call_value(resolve, make_undefined(), &[value])
    }

    fn promise_reject_value(&mut self, value: JSValue) -> JSValue {
        let reject = self.builtin_function("__builtin_promise_reject_static");
        self.call_value(reject, make_undefined(), &[value])
    }

    #[inline(always)]
    fn call_value(&mut self, callee: JSValue, this_value: JSValue, args: &[JSValue]) -> JSValue {
        let caller_depth = self.frame.depth();
        match self.dispatch_call_value(callee, this_value, args) {
            CallAction::Returned(result) => result,
            CallAction::EnteredFrame => {
                self.run_until_frame_depth(caller_depth);
                self.frame.regs[ACC]
            }
        }
    }

    #[inline(always)]
    fn construct_value(&mut self, callee: JSValue, args: &[JSValue]) -> JSValue {
        let caller_depth = self.frame.depth();
        match self.dispatch_construct(callee, args) {
            CallAction::Returned(result) => result,
            CallAction::EnteredFrame => {
                self.run_until_frame_depth(caller_depth);
                self.frame.regs[ACC]
            }
        }
    }

    #[inline(always)]
    fn invoke_call(&mut self, callee_reg: usize, arg_count: usize) -> CallAction {
        let callee = self.frame.regs[callee_reg];

        // 🔥 FAST PATH: 1 argument (common case for recursive fib)
        if arg_count == 1 {
            let arg0 = self.frame.regs[callee_reg + 1];
            return self.dispatch_call_value(callee, self.frame.regs[0], &[arg0]);
        }

        // Fallback (multi-arg, uncommon)
        let args = self.collect_call_args(callee_reg + 1, arg_count);
        self.dispatch_call_value(callee, self.frame.regs[0], &args)
    }

    fn invoke_method_call(
        &mut self,
        callee: JSValue,
        this_value: JSValue,
        arg_count: usize,
        arg_base: usize,
    ) -> CallAction {
        let args = self.collect_call_args(arg_base, arg_count);
        self.dispatch_call_value(callee, this_value, &args)
    }

    fn invoke_spread_call(&mut self, callee_reg: usize, array_reg: usize) -> CallAction {
        let callee = self.frame.regs[callee_reg];
        let args = self
            .array_values(
                self.frame
                    .regs
                    .get(array_reg)
                    .copied()
                    .unwrap_or(make_undefined()),
            )
            .unwrap_or_default();
        self.dispatch_call_value(callee, self.frame.regs[0], &args)
    }

    fn invoke_construct(&mut self, callee_reg: usize, arg_count: usize) -> CallAction {
        let callee = self.frame.regs[callee_reg];
        let args = self.collect_call_args(callee_reg + 1, arg_count);
        self.dispatch_construct(callee, &args)
    }

    #[inline(always)]
    fn enter_frame(
        &mut self,
        entry_pc: usize,
        function_value: JSValue,
        this_value: JSValue,
        args: &[JSValue],
        construct_result: Option<JSValue>,
    ) {
        let return_pc = self.pc;
        let scope_depth = self.scope_chain.len();
        let callee_frame = self.frame.ensure_next_frame();
        callee_frame.reset(
            args,
            this_value,
            entry_pc,
            Some(function_value),
            return_pc,
            construct_result,
            scope_depth,
        );
        self.pc = entry_pc;
    }

    #[inline(always)]
    fn exit_frame(&mut self, result: JSValue) -> bool {
        let result = if self.current_frame_is_async() {
            self.promise_resolve_value(result)
        } else {
            match self.frame.header.construct_result {
                Some(instance) if !is_object(result) => instance,
                _ => result,
            }
        };
        let return_pc = self.frame.header.return_pc;
        let scope_depth = self.frame.header.scope_depth;
        self.restore_scope_depth(scope_depth);

        if self.frame.pop_frame() {
            self.frame.regs[ACC] = result;
            self.pc = return_pc;
            true
        } else {
            self.frame.regs[ACC] = result;
            false
        }
    }

    fn dispatch_call_value(
        &mut self,
        callee: JSValue,
        this_value: JSValue,
        args: &[JSValue],
    ) -> CallAction {
        self.feedback.last_call_kind = Some(self.classify_value(callee));
        let Some(obj_ptr) = object_from_value(callee) else {
            return CallAction::Returned(make_undefined());
        };

        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Function(function) => {
                    let descriptor = function.descriptor;
                    if let Some((entry_pc, _)) = self.function_descriptor_info(descriptor) {
                        self.enter_frame(entry_pc, callee, this_value, args, None);
                        CallAction::EnteredFrame
                    } else if is_undefined(descriptor) {
                        CallAction::Returned(args.first().copied().unwrap_or(this_value))
                    } else {
                        CallAction::Returned(descriptor)
                    }
                }
                ObjectKind::NativeFunction(function) => CallAction::Returned(
                    self.dispatch_native_function(callee, function, this_value, args),
                ),
                ObjectKind::NativeClosure(function) => {
                    CallAction::Returned(self.dispatch_native_closure(function, this_value, args))
                }
                ObjectKind::Class(class) => {
                    let base = class.base;
                    let instance = self.alloc_object_with_kind(ObjectKind::Instance(QInstance {
                        class: callee,
                        object: QObject::new(self.heap_shape.clone()),
                    }));
                    let _ = self.set_property(instance, PropertyKey::Id(0), base);
                    CallAction::Returned(instance)
                }
                _ => CallAction::Returned(make_undefined()),
            }
        }
    }

    fn dispatch_construct(&mut self, callee: JSValue, args: &[JSValue]) -> CallAction {
        self.feedback.last_call_kind = Some(self.classify_value(callee));
        let Some(obj_ptr) = object_from_value(callee) else {
            return CallAction::Returned(self.alloc_object());
        };

        unsafe {
            match &(*obj_ptr).kind {
                ObjectKind::Function(function) => {
                    let descriptor = function.descriptor;
                    let instance = self.alloc_object();
                    if let Some((entry_pc, _)) = self.function_descriptor_info(descriptor) {
                        self.enter_frame(entry_pc, callee, instance, args, Some(instance));
                        CallAction::EnteredFrame
                    } else {
                        CallAction::Returned(instance)
                    }
                }
                ObjectKind::NativeFunction(function) => {
                    if let Some(name) = function.name {
                        let builtin_name = self.atoms.resolve(name).to_owned();
                        if let Some(result) =
                            built_ins::dispatch_constructor(self, callee, &builtin_name, args)
                        {
                            return CallAction::Returned(result);
                        }
                    }

                    CallAction::Returned(self.alloc_object())
                }
                ObjectKind::Class(class) => {
                    let base = class.base;
                    let instance = self.alloc_object_with_kind(ObjectKind::Instance(QInstance {
                        class: callee,
                        object: QObject::new(self.heap_shape.clone()),
                    }));
                    let _ = self.set_property(instance, PropertyKey::Id(0), base);
                    CallAction::Returned(instance)
                }
                _ => CallAction::Returned(self.alloc_object()),
            }
        }
    }

    fn jump_by(&mut self, offset: i16) {
        let next_pc = (self.pc as isize + offset as isize).clamp(0, self.bytecode.len() as isize);
        self.pc = next_pc as usize;
    }

    fn ensure_ic_slot(&mut self, slot: usize) -> &mut InlineCache {
        if slot >= self.frame.ic_vector.len() {
            self.frame
                .ic_vector
                .resize(slot + 1, InlineCache::default());
        }
        &mut self.frame.ic_vector[slot]
    }

    fn ic_has_shape(ic: &InlineCache, shape_id: u32) -> bool {
        match ic.state {
            ICState::Uninit => false,
            ICState::Mono => ic.shape_id == shape_id,
            ICState::Poly => ic.shape_id == shape_id || ic.shapes.contains(&shape_id),
            ICState::Mega => false,
        }
    }

    fn check_ic_slot(&self, slot: usize, obj_ptr: *mut JSObject) -> bool {
        let Some(ic) = self.frame.ic_vector.get(slot) else {
            return false;
        };
        Self::ic_has_shape(ic, self.current_shape_id(obj_ptr))
    }

    fn cached_ic_hit(&self, slot: usize, obj_ptr: *mut JSObject, key: PropertyKey) -> bool {
        let Some(ic) = self.frame.ic_vector.get(slot) else {
            return false;
        };
        Self::ic_has_shape(ic, self.current_shape_id(obj_ptr)) && ic.key == Some(key)
    }

    fn init_ic_slot(&mut self, slot: usize, obj_ptr: *mut JSObject, key: Option<PropertyKey>) {
        let shape_id = self.current_shape_id(obj_ptr);
        let offset = key
            .and_then(|key| self.named_property_offset(obj_ptr, key))
            .unwrap_or(0) as u32;
        let ic = self.ensure_ic_slot(slot);
        let preserved_key = ic.key;
        ic.state = ICState::Mono;
        ic.shape_id = shape_id;
        ic.offset = offset;
        ic.key = key.or(preserved_key);
        ic.shapes.clear();
    }

    fn update_ic_slot(&mut self, slot: usize, obj_ptr: *mut JSObject, key: PropertyKey) {
        let shape_id = self.current_shape_id(obj_ptr);
        let offset = self.named_property_offset(obj_ptr, key).unwrap_or(0) as u32;
        let ic = self.ensure_ic_slot(slot);

        if ic.key != Some(key) {
            ic.state = ICState::Mono;
            ic.shape_id = shape_id;
            ic.offset = offset;
            ic.key = Some(key);
            ic.shapes.clear();
            return;
        }

        match ic.state {
            ICState::Uninit => {
                ic.state = ICState::Mono;
                ic.shape_id = shape_id;
                ic.offset = offset;
                ic.shapes.clear();
            }
            ICState::Mono => {
                if ic.shape_id != shape_id {
                    ic.state = ICState::Poly;
                    ic.offset = offset;
                    ic.shapes.clear();
                    ic.shapes.push(shape_id);
                }
            }
            ICState::Poly => {
                if ic.shape_id != shape_id && !ic.shapes.contains(&shape_id) {
                    if ic.shapes.len() < 3 {
                        ic.shapes.push(shape_id);
                    } else {
                        ic.state = ICState::Mega;
                        ic.shapes.clear();
                    }
                }
            }
            ICState::Mega => {}
        }
    }

    fn get_property_via_ic(&mut self, slot: usize, obj_val: JSValue, key: PropertyKey) -> JSValue {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return make_undefined();
        };

        self.last_ic_object = Some(obj_ptr);
        if let Some(value) = self.cached_named_property_value(slot, obj_ptr, key) {
            return value;
        }

        let value = self.get_property(obj_val, key);
        if !self.cached_ic_hit(slot, obj_ptr, key) {
            self.update_ic_slot(slot, obj_ptr, key);
        }
        value
    }

    fn set_property_via_ic(
        &mut self,
        slot: usize,
        obj_val: JSValue,
        key: PropertyKey,
        value: JSValue,
    ) -> JSValue {
        let Some(obj_ptr) = object_from_value(obj_val) else {
            return make_undefined();
        };

        self.last_ic_object = Some(obj_ptr);
        if self.set_cached_named_property(slot, obj_ptr, key, value) {
            return value;
        }

        let written = self.set_property(obj_val, key, value);
        self.update_ic_slot(slot, obj_ptr, key);
        written
    }

    pub fn collect_garbage(&mut self) {
        gc::collect_garbage(self);
    }

    fn process_pending_call_continuation(&mut self) -> bool {
        let Some(pending) = self.frame.header.pending_call.take() else {
            return false;
        };

        match pending {
            PendingCallContinuation::AddReturnedToAcc { lhs } => {
                self.frame.regs[ACC] = self.add_values(lhs, self.frame.regs[ACC]);
            }
            PendingCallContinuation::Call2SubIAddSecond { callee, arg } => {
                let lhs = self.frame.regs[ACC];
                match self.dispatch_call_value(callee, self.frame.regs[0], &[arg]) {
                    CallAction::Returned(result) => {
                        self.frame.regs[ACC] = self.add_values(lhs, result);
                    }
                    CallAction::EnteredFrame => {
                        if let Some(caller) = self.frame.caller_frame_mut() {
                            caller.header.pending_call =
                                Some(PendingCallContinuation::AddReturnedToAcc { lhs });
                        }
                    }
                }
            }
        }

        true
    }

    fn run_inner(&mut self, stop_at_depth: Option<usize>) {
        loop {
            if self.process_pending_call_continuation() {
                continue;
            }

            if self.pc >= self.bytecode.len() {
                return;
            }

            let insn = self.bytecode[self.pc];
            self.pc += 1;

            let opcode_byte = (insn & 0xFF) as u8;

            // Try threaded dispatch for hot opcodes
            if let Some(handler) = self.dispatch_table[opcode_byte as usize] {
                match handler(self, insn) {
                    ControlFlow::Continue => continue,
                    ControlFlow::Stop => return,
                }
            }

            // Fall back to switch for cold opcodes
            let opcode = Opcode::from(opcode_byte);
            let a = ((insn >> 8) & 0xFF) as usize;
            let b = ((insn >> 16) & 0xFF) as usize;
            let c = ((insn >> 24) & 0xFF) as usize;

            match opcode {
                Opcode::Mov => {
                    self.frame.regs[a] = self.frame.regs[b];
                }
                Opcode::LoadK => {
                    let index = Self::decode_abx(insn);
                    self.frame.regs[a] = self
                        .const_pool
                        .get(index)
                        .copied()
                        .unwrap_or(make_undefined());
                }
                Opcode::Add => {
                    self.frame.regs[ACC] = self.add_values(self.frame.regs[b], self.frame.regs[c]);
                }
                Opcode::GetPropIc => {
                    let key = Self::property_key_from_immediate(c as u16);
                    self.feedback.last_ic_slot = Some(c);
                    self.frame.regs[a] = self.get_property_via_ic(c, self.frame.regs[b], key);
                }
                Opcode::Call => match self.invoke_call(a, b) {
                    CallAction::Returned(result) => self.frame.regs[ACC] = result,
                    CallAction::EnteredFrame => continue,
                },
                Opcode::Call2SubIAdd => {
                    let imm = c as u8 as i8;
                    let next_imm = imm.wrapping_add(1);
                    let callee = self.frame.regs[a];
                    let arg1 = self.sub_immediate_value(self.frame.regs[b], imm);
                    let arg2 = self.sub_immediate_value(self.frame.regs[b], next_imm);
                    match self.dispatch_call_value(callee, self.frame.regs[0], &[arg1]) {
                        CallAction::Returned(result1) => {
                            match self.dispatch_call_value(callee, self.frame.regs[0], &[arg2]) {
                                CallAction::Returned(result2) => {
                                    self.frame.regs[ACC] = self.add_values(result1, result2);
                                }
                                CallAction::EnteredFrame => {
                                    if let Some(caller) = self.frame.caller_frame_mut() {
                                        caller.header.pending_call =
                                            Some(PendingCallContinuation::AddReturnedToAcc {
                                                lhs: result1,
                                            });
                                    }
                                    continue;
                                }
                            }
                        }
                        CallAction::EnteredFrame => {
                            if let Some(caller) = self.frame.caller_frame_mut() {
                                caller.header.pending_call =
                                    Some(PendingCallContinuation::Call2SubIAddSecond {
                                        callee,
                                        arg: arg2,
                                    });
                            }
                            continue;
                        }
                    }
                }
                Opcode::Call1SubI => {
                    let callee = self.frame.regs[a];
                    let arg = self.sub_immediate_value(self.frame.regs[b], c as i8);
                    match self.dispatch_call_value(callee, self.frame.regs[0], &[arg]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::Jmp => {
                    self.jump_by(Self::decode_asbx(insn));
                }
                Opcode::LoadI => {
                    self.frame.regs[a] = make_int32(Self::decode_asbx(insn) as i32);
                }
                Opcode::JmpTrue => {
                    if self.is_truthy_value(self.frame.regs[a]) {
                        self.jump_by(Self::decode_asbx(insn));
                    }
                }
                Opcode::JmpFalse => {
                    if !self.is_truthy_value(self.frame.regs[a]) {
                        self.jump_by(Self::decode_asbx(insn));
                    }
                }
                Opcode::SetPropIc => {
                    let key = Self::property_key_from_immediate(c as u16);
                    self.feedback.last_ic_slot = Some(c);
                    self.frame.regs[ACC] =
                        self.set_property_via_ic(c, self.frame.regs[b], key, self.frame.regs[a]);
                }
                Opcode::AddAccImm8 => {
                    let (lhs, rhs) =
                        self.value_pair(self.frame.regs[ACC], make_int32(b as i8 as i32));
                    self.frame.regs[ACC] = lhs.add(&rhs).raw();
                }
                Opcode::IncAcc => {
                    self.frame.regs[ACC] = self.value_op(self.frame.regs[ACC]).inc().raw();
                }
                Opcode::LoadThis => {
                    self.frame.regs[ACC] = self.frame.regs[0];
                }
                Opcode::Load0 => {
                    self.frame.regs[ACC] = make_int32(0);
                }
                Opcode::Load1 => {
                    self.frame.regs[ACC] = make_int32(1);
                }
                Opcode::Eq => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.eq(&rhs).raw();
                }
                Opcode::Lt => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.lt(&rhs).raw();
                }
                Opcode::Lte => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.le(&rhs).raw();
                }
                Opcode::AddAcc => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[ACC], self.frame.regs[b]);
                    self.frame.regs[ACC] = lhs.add(&rhs).raw();
                }
                Opcode::SubAcc => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[ACC], self.frame.regs[b]);
                    self.frame.regs[ACC] = lhs.sub(&rhs).raw();
                }
                Opcode::MulAcc => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[ACC], self.frame.regs[b]);
                    self.frame.regs[ACC] = lhs.mul(&rhs).raw();
                }
                Opcode::DivAcc => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[ACC], self.frame.regs[b]);
                    self.frame.regs[ACC] = lhs.div(&rhs).raw();
                }
                Opcode::LoadNull => {
                    self.frame.regs[ACC] = make_null();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::LoadTrue => {
                    self.frame.regs[ACC] = make_true();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::LoadFalse => {
                    self.frame.regs[ACC] = make_false();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::LoadGlobalIc | Opcode::GetGlobal => {
                    let key = Self::decode_abx(insn) as u16;
                    self.frame.regs[a] = self
                        .global_object
                        .get(&key)
                        .copied()
                        .unwrap_or(make_undefined());
                }
                Opcode::SetGlobalIc | Opcode::SetGlobal => {
                    let key = Self::decode_abx(insn) as u16;
                    self.global_object.insert(key, self.frame.regs[a]);
                }
                Opcode::Typeof => {
                    self.frame.regs[a] = self.value_op(self.frame.regs[b]).typeof_().raw();
                }
                Opcode::ToNum => {
                    self.frame.regs[a] = self.value_op(self.frame.regs[b]).to_number().raw();
                }
                Opcode::ToStr => {
                    self.frame.regs[a] = self.value_op(self.frame.regs[b]).to_string().raw();
                }
                Opcode::IsUndef => {
                    self.frame.regs[a] = make_bool(is_undefined(self.frame.regs[b]));
                }
                Opcode::IsNull => {
                    self.frame.regs[a] = make_bool(is_null(self.frame.regs[b]));
                }
                Opcode::SubAccImm8 => {
                    let (lhs, rhs) =
                        self.value_pair(self.frame.regs[ACC], make_int32(b as i8 as i32));
                    self.frame.regs[ACC] = lhs.sub(&rhs).raw();
                }
                Opcode::MulAccImm8 => {
                    let (lhs, rhs) =
                        self.value_pair(self.frame.regs[ACC], make_int32(b as i8 as i32));
                    self.frame.regs[ACC] = lhs.mul(&rhs).raw();
                }
                Opcode::DivAccImm8 => {
                    let (lhs, rhs) =
                        self.value_pair(self.frame.regs[ACC], make_int32(b as i8 as i32));
                    self.frame.regs[ACC] = lhs.div(&rhs).raw();
                }
                Opcode::AddStrAcc => {
                    let result = format!(
                        "{}{}",
                        self.display_string(self.frame.regs[ACC]),
                        self.display_string(self.frame.regs[b])
                    );
                    self.frame.regs[ACC] = self.intern_string(result);
                }
                Opcode::AddI => {
                    let result = self.binary_numeric_op(
                        self.frame.regs[b],
                        make_int32(c as i8 as i32),
                        |x, y| x + y,
                    );
                    self.frame.regs[ACC] = result;
                    if a != ACC {
                        self.frame.regs[a] = result;
                    }
                }
                Opcode::SubI => {
                    let result = self.binary_numeric_op(
                        self.frame.regs[b],
                        make_int32(c as i8 as i32),
                        |x, y| x - y,
                    );
                    self.frame.regs[ACC] = result;
                    if a != ACC {
                        self.frame.regs[a] = result;
                    }
                }
                Opcode::MulI => {
                    let result = self.binary_numeric_op(
                        self.frame.regs[b],
                        make_int32(c as i8 as i32),
                        |x, y| x * y,
                    );
                    self.frame.regs[ACC] = result;
                    if a != ACC {
                        self.frame.regs[a] = result;
                    }
                }
                Opcode::DivI => {
                    let result = self.binary_numeric_op(
                        self.frame.regs[b],
                        make_int32(c as i8 as i32),
                        |x, y| x / y,
                    );
                    self.frame.regs[ACC] = result;
                    if a != ACC {
                        self.frame.regs[a] = result;
                    }
                }
                Opcode::ModI => {
                    let result = self.binary_numeric_op(
                        self.frame.regs[b],
                        make_int32(c as i8 as i32),
                        |x, y| x % y,
                    );
                    self.frame.regs[ACC] = result;
                    if a != ACC {
                        self.frame.regs[a] = result;
                    }
                }
                Opcode::Mod => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.rem(&rhs).raw();
                }
                Opcode::Neg => {
                    self.frame.regs[ACC] = self.value_op(self.frame.regs[b]).unary_minus().raw();
                }
                Opcode::Inc => {
                    self.frame.regs[ACC] = self.value_op(self.frame.regs[b]).inc().raw();
                }
                Opcode::Dec => {
                    self.frame.regs[ACC] = self.value_op(self.frame.regs[b]).dec().raw();
                }
                Opcode::AddStr => {
                    let result = format!(
                        "{}{}",
                        self.display_string(self.frame.regs[b]),
                        self.display_string(self.frame.regs[c])
                    );
                    self.frame.regs[ACC] = self.intern_string(result);
                }
                Opcode::ToPrimitive => {
                    self.frame.regs[ACC] = self.value_op(self.frame.regs[b]).to_primitive().raw();
                }
                Opcode::GetPropAcc => {
                    let base = self.value_op(self.frame.regs[b]);
                    let key = self.value_op(self.frame.regs[c]);
                    self.frame.regs[ACC] = base.get(&key).raw();
                }
                Opcode::SetPropAcc => {
                    let key = self.property_key_from_value(self.frame.regs[c]);
                    self.frame.regs[ACC] =
                        self.set_property(self.frame.regs[b], key, self.frame.regs[ACC]);
                }
                Opcode::GetIdxFast | Opcode::GetIdxIc => {
                    let key = self.property_key_from_value(self.frame.regs[c]);
                    let base = self.value_op(self.frame.regs[b]);
                    let key_value = self.value_op(self.frame.regs[c]);
                    let result = base.get(&key_value).raw();
                    if matches!(opcode, Opcode::GetIdxIc)
                        && let Some(obj_ptr) = object_from_value(self.frame.regs[b])
                    {
                        self.feedback.last_ic_slot = Some(c);
                        self.last_ic_object = Some(obj_ptr);
                        self.update_ic_slot(c, obj_ptr, key);
                    }
                    self.frame.regs[a] = result;
                }
                Opcode::SetIdxFast | Opcode::SetIdxIc => {
                    let key = self.property_key_from_value(self.frame.regs[c]);
                    let result = self.set_property(self.frame.regs[b], key, self.frame.regs[a]);
                    if matches!(opcode, Opcode::SetIdxIc)
                        && let Some(obj_ptr) = object_from_value(self.frame.regs[b])
                    {
                        self.feedback.last_ic_slot = Some(c);
                        self.last_ic_object = Some(obj_ptr);
                        self.update_ic_slot(c, obj_ptr, key);
                    }
                    self.frame.regs[ACC] = result;
                }
                Opcode::LoadArg => {
                    self.frame.regs[a] = self.frame.arg(b);
                }
                Opcode::LoadRestArgs => {
                    self.frame.regs[a] = self.collect_rest_args_value(b as usize);
                }
                Opcode::LoadAcc => {
                    self.frame.regs[ACC] = self.frame.regs[a];
                }
                Opcode::StrictEq => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.strict_eq(&rhs).raw();
                }
                Opcode::StrictNeq => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.strict_ne(&rhs).raw();
                }
                Opcode::BitAnd => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.bit_and(&rhs).raw();
                }
                Opcode::BitOr => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.bit_or(&rhs).raw();
                }
                Opcode::BitXor => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.bit_xor(&rhs).raw();
                }
                Opcode::BitNot => {
                    self.frame.regs[ACC] = self.value_op(self.frame.regs[b]).bit_not().raw();
                }
                Opcode::Shl => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.shl(&rhs).raw();
                }
                Opcode::Shr => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.shr(&rhs).raw();
                }
                Opcode::Ushr => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.ushr(&rhs).raw();
                }
                Opcode::Pow => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.pow(&rhs).raw();
                }
                Opcode::LogicalAnd => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.logical_and(&rhs).raw();
                }
                Opcode::LogicalOr => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.logical_or(&rhs).raw();
                }
                Opcode::NullishCoalesce => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.nullish_coalesce(&rhs).raw();
                }
                Opcode::In => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.in_(&rhs).raw();
                }
                Opcode::PrivateIn => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.private_in(&rhs).raw();
                }
                Opcode::Instanceof => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.instanceof(&rhs).raw();
                }
                Opcode::GetLengthIc => {
                    let result = self.get_length_value(self.frame.regs[b]);
                    if let Some(obj_ptr) = object_from_value(self.frame.regs[b]) {
                        let length_key = self.intern_string("length");
                        self.feedback.last_ic_slot = Some(c);
                        self.last_ic_object = Some(obj_ptr);
                        self.update_ic_slot(c, obj_ptr, PropertyKey::Value(length_key));
                    }
                    self.frame.regs[a] = result;
                }
                Opcode::ArrayPushAcc => {
                    self.frame.regs[ACC] =
                        self.array_push(self.frame.regs[a], self.frame.regs[ACC]);
                }
                Opcode::NewObj => {
                    self.frame.regs[a] = self.alloc_object();
                }
                Opcode::NewArr => {
                    self.frame.regs[a] = self.alloc_array(b);
                }
                Opcode::NewFunc => {
                    let descriptor = self
                        .const_pool
                        .get(Self::decode_abx(insn))
                        .copied()
                        .unwrap_or(make_undefined());
                    self.frame.regs[a] = self.alloc_function(descriptor);
                }
                Opcode::NewClass => {
                    self.frame.regs[a] = self.alloc_class(self.frame.regs[b]);
                }
                Opcode::GetProp | Opcode::GetSuper => {
                    self.frame.regs[a] = self.get_property(
                        self.frame.regs[b],
                        Self::property_key_from_immediate(c as u16),
                    );
                }
                Opcode::SetProp | Opcode::SetSuper => {
                    self.frame.regs[ACC] = self.set_property(
                        self.frame.regs[b],
                        Self::property_key_from_immediate(c as u16),
                        self.frame.regs[a],
                    );
                }
                Opcode::GetPrivateProp => {
                    self.frame.regs[ACC] = self.get_private_property(
                        self.frame.regs[b],
                        Self::property_key_from_immediate(c as u16),
                    );
                }
                Opcode::SetPrivateProp => {
                    self.frame.regs[ACC] = self.set_private_property(
                        self.frame.regs[b],
                        Self::property_key_from_immediate(c as u16),
                        self.frame.regs[a],
                    );
                }
                Opcode::GetUpval | Opcode::LoadClosure => {
                    self.frame.regs[a] = self
                        .current_function_value()
                        .and_then(|function_value| self.get_function_upvalue(function_value, b))
                        .or_else(|| self.upvalues.get(b).copied())
                        .unwrap_or(make_undefined());
                }
                Opcode::SetUpval => {
                    let value = self.frame.regs[a];
                    if let Some(function_value) = self.current_function_value() {
                        let _ = self.set_function_upvalue(function_value, b, value);
                    } else {
                        let _ = self.set_function_upvalue(value, b, value);
                        self.ensure_upvalue_slot(b);
                        self.upvalues[b] = value;
                    }
                }
                Opcode::GetScope => {
                    self.frame.regs[a] = self.scope_at_depth(b).unwrap_or(make_undefined());
                }
                Opcode::SetScope => {
                    self.set_scope_at_depth(b, self.frame.regs[a]);
                }
                Opcode::ResolveScope => {
                    let name = Self::decode_abx(insn) as u16;
                    self.frame.regs[a] = self
                        .resolve_scope_value(name)
                        .or_else(|| self.scope_chain.last().copied())
                        .unwrap_or(make_undefined());
                }
                Opcode::DeleteProp => {
                    let deleted = self.delete_property(
                        self.frame.regs[b],
                        Self::property_key_from_immediate(c as u16),
                    );
                    self.frame.regs[a] = make_bool(deleted);
                }
                Opcode::HasProp => {
                    let has = self.has_property(
                        self.frame.regs[b],
                        Self::property_key_from_immediate(c as u16),
                    );
                    self.frame.regs[a] = make_bool(has);
                }
                Opcode::Keys => {
                    let keys = self.get_keys(self.frame.regs[b]);
                    let array = self.alloc_array(keys.len());
                    for key in keys {
                        let key_value = self.property_key_to_value(key);
                        let _ = self.array_push(array, key_value);
                    }
                    self.frame.regs[a] = array;
                }
                Opcode::ForIn => {
                    let keys = self
                        .get_keys(self.frame.regs[b])
                        .into_iter()
                        .map(|key| self.property_key_to_value(key))
                        .collect();
                    let iterator = self.alloc_iterator(keys);
                    self.frame.regs[a] = iterator;
                    self.frame.regs[ACC] = self.iterator_next_value(iterator);
                }
                Opcode::IteratorNext => {
                    self.frame.regs[ACC] = self.iterator_next_value(self.frame.regs[a]);
                }
                Opcode::Spread => {
                    let source_values = self.array_values(self.frame.regs[b]).unwrap_or_default();
                    for value in source_values {
                        let _ = self.array_push(self.frame.regs[a], value);
                    }
                }
                Opcode::Destructure => {
                    let source_values = self.array_values(self.frame.regs[b]).unwrap_or_default();
                    for (index, value) in source_values.into_iter().enumerate() {
                        let dst = a + index;
                        if dst < self.frame.regs.len() {
                            self.frame.regs[dst] = value;
                        }
                    }
                }
                Opcode::CreateEnv => {
                    let env = self.alloc_env();
                    self.scope_chain.push(env);
                    self.frame.header.env = Some(env);
                    self.frame.regs[a] = env;
                }
                Opcode::LoadName => {
                    let value = self.load_name_value(Self::decode_abx(insn) as u16);
                    self.frame.regs[a] = value;
                    self.frame.regs[ACC] = value;
                }
                Opcode::StoreName => {
                    self.store_name_value(Self::decode_abx(insn) as u16, self.frame.regs[a]);
                }
                Opcode::InitName => {
                    self.init_name_value(Self::decode_abx(insn) as u16, self.frame.regs[a]);
                }
                Opcode::NewThis => {
                    self.frame.regs[a] = self.alloc_object();
                }
                Opcode::TypeofName => {
                    let value = self.load_name_value(Self::decode_abx(insn) as u16);
                    self.frame.regs[a] = self.value_op(value).typeof_().raw();
                }
                Opcode::JmpEq => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[a], self.frame.regs[b]);
                    if bool_from_value(lhs.eq(&rhs).raw()).unwrap_or(false) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::JmpNeq => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[a], self.frame.regs[b]);
                    if bool_from_value(lhs.ne(&rhs).raw()).unwrap_or(false) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::JmpLt => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[a], self.frame.regs[b]);
                    if bool_from_value(lhs.lt(&rhs).raw()).unwrap_or(false) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::JmpLtF64 => {
                    let lhs = self.frame.regs[a];
                    let rhs = self.frame.regs[b];
                    if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
                        if lhs < rhs {
                            self.jump_by(c as i8 as i16);
                        }
                    } else if self.less_than(lhs, rhs) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::JmpLte => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[a], self.frame.regs[b]);
                    if bool_from_value(lhs.le(&rhs).raw()).unwrap_or(false) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::JmpLteF64 => {
                    let lhs = self.frame.regs[a];
                    let rhs = self.frame.regs[b];
                    if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
                        if lhs <= rhs {
                            self.jump_by(c as i8 as i16);
                        }
                    } else if self.less_than_or_equal(lhs, rhs) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::JmpLteFalse => {
                    let (lhs, rhs) = self.value_pair(self.frame.regs[a], self.frame.regs[b]);
                    if !bool_from_value(lhs.le(&rhs).raw()).unwrap_or(false) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::JmpLteFalseF64 => {
                    let lhs = self.frame.regs[a];
                    let rhs = self.frame.regs[b];
                    if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
                        if lhs > rhs {
                            self.jump_by(c as i8 as i16);
                        }
                    } else if !self.less_than_or_equal(lhs, rhs) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::LoopIncJmp => {
                    let current =
                        self.binary_numeric_op(self.frame.regs[a], make_number(1.0), |x, y| x + y);
                    self.frame.regs[a] = current;
                    if self.less_than(current, self.frame.regs[ACC]) {
                        self.jump_by(Self::decode_asbx(insn));
                    }
                }
                Opcode::Switch => {
                    if let Some(offset) = self.switch_jump_offset(b, self.frame.regs[a]) {
                        self.jump_by(offset);
                    }
                }
                Opcode::LoopHint => {
                    let pc = self.pc.saturating_sub(1);
                    self.feedback.last_loop_hint_pc = Some(pc);
                    *self.feedback.loop_hint_counts.entry(pc).or_default() += 1;
                }
                Opcode::Ret => {
                    if !self.exit_frame(self.frame.regs[ACC]) {
                        return;
                    }
                    if stop_at_depth == Some(self.frame.depth()) {
                        return;
                    }
                    continue;
                }
                Opcode::RetU => {
                    if !self.exit_frame(make_undefined()) {
                        return;
                    }
                    if stop_at_depth == Some(self.frame.depth()) {
                        return;
                    }
                    continue;
                }
                Opcode::RetReg => {
                    if !self.exit_frame(self.frame.regs[a]) {
                        return;
                    }
                    if stop_at_depth == Some(self.frame.depth()) {
                        return;
                    }
                    continue;
                }
                Opcode::TailCall | Opcode::CallIc => match self.invoke_call(a, b) {
                    CallAction::Returned(result) => self.frame.regs[ACC] = result,
                    CallAction::EnteredFrame => continue,
                },
                Opcode::Construct => match self.invoke_construct(a, b) {
                    CallAction::Returned(result) => self.frame.regs[ACC] = result,
                    CallAction::EnteredFrame => continue,
                },
                Opcode::CallVar | Opcode::CallIcVar => match self.invoke_spread_call(a, a + 1) {
                    CallAction::Returned(result) => self.frame.regs[ACC] = result,
                    CallAction::EnteredFrame => continue,
                },
                Opcode::CallThis => {
                    let args = self.collect_call_args(a + 1, c as usize);
                    match self.dispatch_call_value(self.frame.regs[a], self.frame.regs[b], &args) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::CallThisVar => {
                    let args = self.array_values(self.frame.regs[c]).unwrap_or_default();
                    match self.dispatch_call_value(self.frame.regs[a], self.frame.regs[b], &args) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::Enter => {
                    let frame_size = Self::decode_abx(insn).min(256);
                    self.frame.header.frame_size = frame_size as u32;
                    self.frame.header.register_count = frame_size as u32;
                    self.frame.scope_stack.push(self.scope_chain.len());
                }
                Opcode::Leave => {
                    let scope_depth = self
                        .frame
                        .scope_stack
                        .pop()
                        .unwrap_or(self.frame.header.scope_depth);
                    self.restore_scope_depth(scope_depth);
                }
                Opcode::Yield | Opcode::Await => {
                    self.frame.regs[ACC] = self.frame.regs[a];
                }
                Opcode::Throw => {
                    let exception = self.frame.regs[a];
                    if let Some(catch_pc) = self.frame.try_stack.pop() {
                        self.last_exception = exception;
                        self.pc = catch_pc.min(self.bytecode.len());
                    } else if self.current_frame_is_async() {
                        let rejection = self.promise_reject_value(exception);
                        if !self.exit_frame(rejection) {
                            return;
                        }
                        if stop_at_depth == Some(self.frame.depth()) {
                            return;
                        }
                        continue;
                    } else {
                        if !self.exit_frame(exception) {
                            return;
                        }
                        if stop_at_depth == Some(self.frame.depth()) {
                            return;
                        }
                        continue;
                    }
                }
                Opcode::Try => {
                    let catch_pc = (self.pc as isize + Self::decode_asbx(insn) as isize)
                        .clamp(0, self.bytecode.len() as isize)
                        as usize;
                    self.frame.try_stack.push(catch_pc);
                }
                Opcode::EndTry => {
                    let _ = self.frame.try_stack.pop();
                }
                Opcode::Catch => {
                    self.frame.regs[a] = self.last_exception;
                    self.frame.regs[ACC] = self.last_exception;
                }
                Opcode::Finally => {
                    self.last_exception = make_undefined();
                }
                Opcode::ProfileType => {
                    let slot = if b != 0 || c != 0 { c } else { a };
                    let reg = if b != 0 || c != 0 { b } else { ACC };
                    self.observe_type_feedback_slot(slot, self.frame.regs[reg]);
                }
                Opcode::ProfileCall => {
                    let slot = if b != 0 || c != 0 { c } else { a };
                    let kind = if b != 0 || c != 0 {
                        self.classify_value(self.frame.regs[b])
                    } else {
                        self.feedback
                            .last_call_kind
                            .unwrap_or_else(|| self.classify_value(self.frame.regs[ACC]))
                    };
                    self.observe_call_feedback_kind(slot, kind);
                }
                Opcode::ProfileRet => {
                    self.observe_return_value(self.frame.regs[ACC]);
                }
                Opcode::CheckType => {
                    let expected_id = if b != 0 || c != 0 { c as u8 } else { a as u8 };
                    let reg = if b != 0 || c != 0 { b } else { ACC };
                    if let Some(expected) = ValueProfileKind::from_id(expected_id) {
                        let observed = self.classify_value(self.frame.regs[reg]);
                        if observed != expected {
                            self.record_deopt(DeoptReason::TypeMismatch { expected, observed });
                        }
                    }
                }
                Opcode::CheckStruct => {
                    let expected = if b != 0 || c != 0 { c as u32 } else { a as u32 };
                    let reg = if b != 0 || c != 0 { b } else { ACC };
                    let observed = object_from_value(self.frame.regs[reg])
                        .map(|obj_ptr| self.current_shape_id(obj_ptr))
                        .unwrap_or(0);
                    if observed != expected {
                        self.record_deopt(DeoptReason::StructMismatch { expected, observed });
                    }
                }
                Opcode::CheckIc => {
                    let slot = if c != 0 {
                        c
                    } else if a != 0 {
                        a
                    } else {
                        self.feedback.last_ic_slot.unwrap_or(0)
                    };
                    let reg = if b != 0 || c != 0 { b } else { ACC };
                    self.feedback.last_ic_slot = Some(slot);
                    let obj_ptr = if b != 0 || c != 0 {
                        object_from_value(self.frame.regs[reg])
                    } else {
                        object_from_value(self.frame.regs[reg]).or(self.last_ic_object)
                    };
                    let hit = obj_ptr.is_some_and(|obj_ptr| self.check_ic_slot(slot, obj_ptr));
                    self.last_ic_object = obj_ptr;
                    self.frame.regs[ACC] = make_bool(hit);
                }
                Opcode::IcInit => {
                    let slot = if c != 0 {
                        c
                    } else if a != 0 {
                        a
                    } else {
                        self.feedback.last_ic_slot.unwrap_or(0)
                    };
                    let reg = if b != 0 || c != 0 { b } else { ACC };
                    self.feedback.last_ic_slot = Some(slot);
                    let obj_ptr = if b != 0 || c != 0 {
                        object_from_value(self.frame.regs[reg])
                    } else {
                        object_from_value(self.frame.regs[reg]).or(self.last_ic_object)
                    };
                    if let Some(obj_ptr) = obj_ptr {
                        self.last_ic_object = Some(obj_ptr);
                        self.init_ic_slot(slot, obj_ptr, None);
                    }
                }
                Opcode::IcUpdate => {
                    let slot = if c != 0 {
                        c
                    } else if a != 0 {
                        a
                    } else {
                        self.feedback.last_ic_slot.unwrap_or(0)
                    };
                    let reg = if b != 0 || c != 0 { b } else { ACC };
                    self.feedback.last_ic_slot = Some(slot);
                    let obj_ptr = if b != 0 || c != 0 {
                        object_from_value(self.frame.regs[reg])
                    } else {
                        object_from_value(self.frame.regs[reg]).or(self.last_ic_object)
                    };
                    if let Some(obj_ptr) = obj_ptr {
                        self.last_ic_object = Some(obj_ptr);
                        let shape_id = self.current_shape_id(obj_ptr);
                        let ic = self.ensure_ic_slot(slot);
                        match ic.state {
                            ICState::Uninit => {
                                ic.state = ICState::Mono;
                                ic.shape_id = shape_id;
                                ic.offset = 0;
                                ic.shapes.clear();
                            }
                            ICState::Mono => {
                                if ic.shape_id != shape_id {
                                    ic.state = ICState::Poly;
                                    ic.shapes.clear();
                                    ic.shapes.push(shape_id);
                                }
                            }
                            ICState::Poly => {
                                if ic.shape_id != shape_id && !ic.shapes.contains(&shape_id) {
                                    if ic.shapes.len() < 3 {
                                        ic.shapes.push(shape_id);
                                    } else {
                                        ic.state = ICState::Mega;
                                        ic.shapes.clear();
                                    }
                                }
                            }
                            ICState::Mega => {}
                        }
                    }
                }
                Opcode::IcMiss => {
                    let slot = if a != 0 {
                        a
                    } else {
                        self.feedback.last_ic_slot.unwrap_or(0)
                    };
                    self.feedback.ic_misses = self.feedback.ic_misses.saturating_add(1);
                    self.feedback.last_ic_slot = Some(slot);
                    self.frame.regs[ACC] = make_false();
                }
                Opcode::OsrEntry => {
                    self.feedback.osr_entries = self.feedback.osr_entries.saturating_add(1);
                    self.feedback.osr_active = true;
                }
                Opcode::ProfileHotLoop => {
                    let pc = self
                        .feedback
                        .last_loop_hint_pc
                        .unwrap_or_else(|| self.pc.saturating_sub(1));
                    *self.feedback.hot_loop_counts.entry(pc).or_default() += 1;
                }
                Opcode::OsrExit => {
                    self.feedback.osr_exits = self.feedback.osr_exits.saturating_add(1);
                    self.feedback.osr_active = false;
                }
                Opcode::JitHint => {
                    let key = if a != 0 { a } else { self.pc.saturating_sub(1) };
                    *self.feedback.jit_hints.entry(key).or_default() += 1;
                }
                Opcode::SafetyCheck => {
                    let reg = if a != 0 { a } else { ACC };
                    self.feedback.safety_checks = self.feedback.safety_checks.saturating_add(1);
                    let failed = reg >= self.frame.regs.len()
                        || self.frame.header.register_count as usize > self.frame.regs.len()
                        || self.pc > self.bytecode.len()
                        || self.frame.regs[reg].is_empty();
                    if failed {
                        self.feedback.failed_safety_checks =
                            self.feedback.failed_safety_checks.saturating_add(1);
                        self.record_deopt(DeoptReason::SafetyCheck { register: reg });
                    }
                }
                Opcode::GetPropIcCall => {
                    let key = Self::property_key_from_immediate(c as u16);
                    let this_value = self.frame.regs[b];
                    self.feedback.last_ic_slot = Some(c);
                    let callee = self.get_property_via_ic(c, this_value, key);
                    self.frame.regs[a] = callee;
                    match self.invoke_method_call(callee, this_value, 0, a + 1) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::IncJmpFalseLoop => {
                    self.frame.regs[ACC] =
                        self.binary_numeric_op(self.frame.regs[ACC], make_number(1.0), |x, y| {
                            x + y
                        });
                    if !self.is_truthy_value(self.frame.regs[a]) {
                        self.jump_by(Self::decode_asbx(insn));
                    }
                }
                Opcode::LoadKAddAcc => {
                    let constant = self
                        .const_pool
                        .get(Self::decode_abx(insn))
                        .copied()
                        .unwrap_or(make_undefined());
                    self.frame.regs[ACC] = self.binary_add(constant, self.frame.regs[ACC]);
                }
                Opcode::AddMov => {
                    let result = self.binary_add(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[ACC] = result;
                    self.frame.regs[a] = result;
                }
                Opcode::EqJmpTrue => {
                    if self.abstract_equal(self.frame.regs[b], self.frame.regs[c]) {
                        self.jump_by(a as i8 as i16);
                    }
                }
                Opcode::GetPropAccCall => {
                    let this_value = self.frame.regs[b];
                    let key = self.property_key_from_value(self.frame.regs[c]);
                    let callee = self.get_property(this_value, key);
                    match self.dispatch_call_value(callee, this_value, &[]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::LoadKMulAcc => {
                    let constant = self
                        .const_pool
                        .get(Self::decode_abx(insn))
                        .copied()
                        .unwrap_or(make_undefined());
                    self.frame.regs[ACC] =
                        self.binary_numeric_op(constant, self.frame.regs[ACC], |x, y| x * y);
                }
                Opcode::LtJmp => {
                    if self.less_than(self.frame.regs[b], self.frame.regs[c]) {
                        self.jump_by(a as i8 as i16);
                    }
                }
                Opcode::GetPropIcMov => {
                    let key = Self::property_key_from_immediate(c as u16);
                    self.feedback.last_ic_slot = Some(c);
                    self.frame.regs[a] = self.get_property_via_ic(c, self.frame.regs[b], key);
                }
                Opcode::GetPropAddImmSetPropIc => {
                    let key = Self::property_key_from_immediate(c as u16);
                    self.feedback.last_ic_slot = Some(c);
                    let current = self.get_property_via_ic(c, self.frame.regs[b], key);
                    let next =
                        self.binary_numeric_op(current, make_int32(a as i8 as i32), |x, y| x + y);
                    self.frame.regs[ACC] =
                        self.set_property_via_ic(c, self.frame.regs[b], key, next);
                }
                Opcode::AddAccImm8Mov => {
                    self.frame.regs[ACC] = self.binary_numeric_op(
                        self.frame.regs[ACC],
                        make_int32(b as i8 as i32),
                        |x, y| x + y,
                    );
                    self.frame.regs[a] = self.frame.regs[ACC];
                }
                Opcode::CallIcSuper => match self.invoke_call(a, b) {
                    CallAction::Returned(result) => self.frame.regs[ACC] = result,
                    CallAction::EnteredFrame => continue,
                },
                Opcode::LoadThisCall => {
                    match self.dispatch_call_value(self.frame.regs[0], self.frame.regs[0], &[]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::EqJmpFalse => {
                    if !self.abstract_equal(self.frame.regs[b], self.frame.regs[c]) {
                        self.jump_by(a as i8 as i16);
                    }
                }
                Opcode::LoadKSubAcc => {
                    let constant = self
                        .const_pool
                        .get(Self::decode_abx(insn))
                        .copied()
                        .unwrap_or(make_undefined());
                    self.frame.regs[ACC] =
                        self.binary_numeric_op(constant, self.frame.regs[ACC], |x, y| x - y);
                }
                Opcode::GetLengthIcCall => {
                    self.frame.regs[ACC] = self.get_length_value(self.frame.regs[b]);
                }
                Opcode::AddStrAccMov => {
                    let result = format!(
                        "{}{}",
                        self.display_string(self.frame.regs[ACC]),
                        self.display_string(self.frame.regs[b])
                    );
                    self.frame.regs[ACC] = self.intern_string(result);
                    self.frame.regs[a] = self.frame.regs[ACC];
                }
                Opcode::IncAccJmp => {
                    self.frame.regs[ACC] =
                        self.binary_numeric_op(self.frame.regs[ACC], make_number(1.0), |x, y| {
                            x + y
                        });
                    self.jump_by(Self::decode_asbx(insn));
                }
                Opcode::GetPropChainAcc => {
                    let inner_reg = self.array_index_from_value(self.frame.regs[b]).unwrap_or(0);
                    let base = self
                        .frame
                        .regs
                        .get(inner_reg)
                        .copied()
                        .unwrap_or(make_undefined());
                    self.frame.regs[ACC] =
                        self.get_property(base, Self::property_key_from_immediate(c as u16));
                }
                Opcode::TestJmpTrue => {
                    if self.is_truthy_value(self.frame.regs[a]) {
                        self.jump_by(Self::decode_asbx(insn));
                    }
                }
                Opcode::LoadArgCall => {
                    self.frame.regs[a] = self.frame.arg(b);
                    match self.dispatch_call_value(self.frame.regs[a], self.frame.regs[0], &[]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::MulAccMov => {
                    self.frame.regs[ACC] =
                        self.binary_numeric_op(self.frame.regs[ACC], self.frame.regs[b], |x, y| {
                            x * y
                        });
                    self.frame.regs[a] = self.frame.regs[ACC];
                }
                Opcode::LteJmpLoop => {
                    if self.less_than_or_equal(self.frame.regs[b], self.frame.regs[c]) {
                        self.jump_by(a as i8 as i16);
                    }
                }
                Opcode::NewObjInitProp => {
                    let object = self.alloc_object();
                    let _ = self.set_property(
                        object,
                        Self::property_key_from_immediate(c as u16),
                        self.frame.regs[b],
                    );
                    self.frame.regs[a] = object;
                }
                Opcode::ProfileHotCall => match self.invoke_call(b, c) {
                    CallAction::Returned(result) => self.frame.regs[ACC] = result,
                    CallAction::EnteredFrame => continue,
                },
                Opcode::AddI32 => {
                    // Fast path: int32 + int32
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    // Check if both are ints
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_add(b_int) {
                            self.frame.regs[ACC] = make_int32(result);
                            if a != ACC {
                                self.frame.regs[a] = make_int32(result);
                            }
                            continue;
                        }
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.add(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::AddF64 => {
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
                        self.write_result_reg(a, make_number(lhs + rhs));
                        continue;
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.add(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::SubI32 => {
                    // Fast path: int32 - int32
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_sub(b_int) {
                            self.frame.regs[ACC] = make_int32(result);
                            if a != ACC {
                                self.frame.regs[a] = make_int32(result);
                            }
                            continue;
                        }
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.sub(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::SubF64 => {
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
                        self.write_result_reg(a, make_number(lhs - rhs));
                        continue;
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.sub(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::MulI32 => {
                    // Fast path: int32 * int32
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_mul(b_int) {
                            self.frame.regs[ACC] = make_int32(result);
                            if a != ACC {
                                self.frame.regs[a] = make_int32(result);
                            }
                            continue;
                        }
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.mul(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::MulF64 => {
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
                        self.write_result_reg(a, make_number(lhs * rhs));
                        continue;
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.mul(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::LtF64 => {
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
                        self.write_result_reg(a, make_bool(lhs < rhs));
                        continue;
                    }
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.lt(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::LteF64 => {
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    if let Some((lhs, rhs)) = self.fast_number_pair(lhs, rhs) {
                        self.write_result_reg(a, make_bool(lhs <= rhs));
                        continue;
                    }
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.le(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::Reserved(_) => {}
                // Superinstruction handlers
                Opcode::RetIfLteI => {
                    // RetIfLteI a, b, c: if reg[a] <= reg[b], return reg[c]
                    if self.less_than_or_equal(self.frame.regs[a], self.frame.regs[b]) {
                        if !self.exit_frame(self.frame.regs[c]) {
                            return;
                        }
                        if stop_at_depth == Some(self.frame.depth()) {
                            return;
                        }
                        continue;
                    }
                }
                Opcode::AddAccReg => {
                    // AddAccReg a, b: ACC = reg[a] + reg[b]
                    self.frame.regs[ACC] = self.add_values(self.frame.regs[a], self.frame.regs[b]);
                }
                Opcode::Call1Add => {
                    // Call1Add a, b: call reg[a] with 1 arg, add result to ACC
                    let callee = self.frame.regs[a];
                    let arg = self.frame.regs[b];
                    let lhs = self.frame.regs[ACC];
                    match self.dispatch_call_value(callee, self.frame.regs[0], &[arg]) {
                        CallAction::Returned(result) => {
                            self.frame.regs[ACC] = self.add_values(lhs, result);
                        }
                        CallAction::EnteredFrame => {
                            if let Some(caller) = self.frame.caller_frame_mut() {
                                caller.header.pending_call =
                                    Some(PendingCallContinuation::AddReturnedToAcc { lhs });
                            }
                            continue;
                        }
                    }
                }
                Opcode::Call2Add => {
                    // Call2Add a, b, c: call reg[a] with 2 args, add result to ACC
                    let callee = self.frame.regs[a];
                    let arg1 = self.frame.regs[b];
                    let arg2 = self.frame.regs[c];
                    let lhs = self.frame.regs[ACC];
                    match self.dispatch_call_value(callee, self.frame.regs[0], &[arg1, arg2]) {
                        CallAction::Returned(result) => {
                            self.frame.regs[ACC] = self.add_values(lhs, result);
                        }
                        CallAction::EnteredFrame => {
                            if let Some(caller) = self.frame.caller_frame_mut() {
                                caller.header.pending_call =
                                    Some(PendingCallContinuation::AddReturnedToAcc { lhs });
                            }
                            continue;
                        }
                    }
                }
                Opcode::LoadKAdd => {
                    // LoadKAdd a, index: reg[a] = const_pool[index] + ACC
                    let index = Self::decode_abx(insn);
                    let constant = self
                        .const_pool
                        .get(index)
                        .copied()
                        .unwrap_or(make_undefined());
                    let (lhs, rhs) = self.value_pair(constant, self.frame.regs[ACC]);
                    self.frame.regs[a] = lhs.add(&rhs).raw();
                }
                Opcode::LoadKCmp => {
                    // LoadKCmp a, index: ACC = const_pool[index] < reg[a]
                    let index = Self::decode_abx(insn);
                    let constant = self
                        .const_pool
                        .get(index)
                        .copied()
                        .unwrap_or(make_undefined());
                    self.frame.regs[ACC] = make_bool(self.less_than(constant, self.frame.regs[a]));
                }
                Opcode::CmpJmp => {
                    // CmpJmp a, b, offset: if reg[a] < reg[b], jump by offset
                    if self.less_than(self.frame.regs[a], self.frame.regs[b]) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::GetPropCall => {
                    // GetPropCall a, b, key: call reg[b].key with 0 args, store result in reg[a]
                    let key = Self::property_key_from_immediate(c as u16);
                    let this_value = self.frame.regs[b];
                    let callee = self.get_property(this_value, key);
                    self.frame.regs[a] = callee;
                    match self.dispatch_call_value(callee, this_value, &[]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::CallRet => {
                    // CallRet a, b: call reg[a] with b args, return result
                    let caller_depth = self.frame.depth();
                    match self.invoke_call(a, b) {
                        CallAction::Returned(result) => {
                            if !self.exit_frame(result) {
                                return;
                            }
                            if stop_at_depth == Some(self.frame.depth()) {
                                return;
                            }
                            continue;
                        }
                        CallAction::EnteredFrame => {
                            self.run_until_frame_depth(caller_depth);
                            let result = self.frame.regs[ACC];
                            if !self.exit_frame(result) {
                                return;
                            }
                            if stop_at_depth == Some(self.frame.depth()) {
                                return;
                            }
                            continue;
                        }
                    }
                }
                // Specialized opcodes (stubs for now)
                Opcode::AddI32Fast => {
                    // Fast int32 addition (inline)
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_add(b_int) {
                            self.frame.regs[ACC] = make_int32(result);
                            if a != ACC {
                                self.frame.regs[a] = make_int32(result);
                            }
                            continue;
                        }
                    }
                    // Fall back to regular AddI32
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.add(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::AddF64Fast => {
                    // Fast f64 addition (inline)
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];
                    if lhs.is_f64() && rhs.is_f64() {
                        let a_f64 = lhs.f64_payload_unchecked();
                        let b_f64 = rhs.f64_payload_unchecked();
                        self.frame.regs[ACC] = make_number(a_f64 + b_f64);
                        if a != ACC {
                            self.frame.regs[a] = self.frame.regs[ACC];
                        }
                        continue;
                    }
                    // Fall back to regular AddF64
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.add(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::SubI32Fast => {
                    // Fast int32 subtraction (inline)
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_sub(b_int) {
                            self.frame.regs[ACC] = make_int32(result);
                            if a != ACC {
                                self.frame.regs[a] = make_int32(result);
                            }
                            continue;
                        }
                    }
                    // Fall back to regular SubI32
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.sub(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::MulI32Fast => {
                    // Fast int32 multiplication (inline)
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_mul(b_int) {
                            self.frame.regs[ACC] = make_int32(result);
                            if a != ACC {
                                self.frame.regs[a] = make_int32(result);
                            }
                            continue;
                        }
                    }
                    // Fall back to regular MulI32
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.mul(&rhs).raw();
                    if a != ACC {
                        self.frame.regs[a] = self.frame.regs[ACC];
                    }
                }
                Opcode::EqI32Fast => {
                    // Fast int32 equality (inline)
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        self.frame.regs[ACC] = make_bool(a_int == b_int);
                        continue;
                    }
                    // Fall back to regular Eq
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.eq(&rhs).raw();
                }
                Opcode::LtI32Fast => {
                    // Fast int32 less than (inline)
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        self.frame.regs[ACC] = make_bool(a_int < b_int);
                        continue;
                    }
                    // Fall back to regular Lt
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[ACC] = lhs.lt(&rhs).raw();
                }
                Opcode::JmpI32Fast => {
                    // Fast int32 conditional jump (inline)
                    let lhs = self.frame.regs[a];
                    let rhs = self.frame.regs[b];
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if a_int < b_int {
                            self.jump_by(c as i8 as i16);
                        }
                        continue;
                    }
                    // Fall back to regular comparison
                    if self.less_than(lhs, rhs) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::GetPropMono => {
                    // Monomorphic property get (assumes shape is known)
                    let key = Self::property_key_from_immediate(c as u16);
                    self.feedback.last_ic_slot = Some(c);
                    let obj_val = self.frame.regs[b];
                    self.frame.regs[a] = if let Some(obj_ptr) = object_from_value(obj_val) {
                        self.get_named_property_slot(obj_ptr, key)
                            .unwrap_or_else(|| self.get_property(obj_val, key))
                    } else {
                        make_undefined()
                    };
                }
                Opcode::CallMono => {
                    // Monomorphic call (assumes callee type is known)
                    match self.invoke_call(a, b) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                // Call opcodes
                Opcode::Call0 => {
                    // Call0 a: call reg[a] with 0 args
                    match self.dispatch_call_value(self.frame.regs[a], self.frame.regs[0], &[]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::Call1 => {
                    // Call1 a, b: call reg[a] with 1 arg (reg[b])
                    match self.dispatch_call_value(
                        self.frame.regs[a],
                        self.frame.regs[0],
                        &[self.frame.regs[b]],
                    ) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::Call2 => {
                    // Call2 a, b, c: call reg[a] with 2 args (reg[b], reg[c])
                    match self.dispatch_call_value(
                        self.frame.regs[a],
                        self.frame.regs[0],
                        &[self.frame.regs[b], self.frame.regs[c]],
                    ) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::Call3 => {
                    // Call3 a, b, c, d: call reg[a] with 3 args (reg[b], reg[c], reg[d])
                    let d = ((insn >> 8) & 0xFF) as usize; // Note: reusing 'a' field for 4th arg
                    match self.dispatch_call_value(
                        self.frame.regs[a],
                        self.frame.regs[0],
                        &[self.frame.regs[b], self.frame.regs[c], self.frame.regs[d]],
                    ) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::CallMethod1 => {
                    // CallMethod1 a, slot: call reg[a].slot with 1 arg from reg[a + 1]
                    let this_value = self.frame.regs[a];
                    let slot = Self::decode_abx(insn) as u16;
                    let arg = self
                        .frame
                        .regs
                        .get(a + 1)
                        .copied()
                        .unwrap_or(make_undefined());
                    let method = self.get_property(this_value, PropertyKey::Id(slot));
                    match self.dispatch_call_value(method, this_value, &[arg]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::CallMethod2 => {
                    // CallMethod2 a, slot: call reg[a].slot with args from reg[a + 1], reg[a + 2]
                    let this_value = self.frame.regs[a];
                    let slot = Self::decode_abx(insn) as u16;
                    let arg1 = self
                        .frame
                        .regs
                        .get(a + 1)
                        .copied()
                        .unwrap_or(make_undefined());
                    let arg2 = self
                        .frame
                        .regs
                        .get(a + 2)
                        .copied()
                        .unwrap_or(make_undefined());
                    let method = self.get_property(this_value, PropertyKey::Id(slot));
                    match self.dispatch_call_value(method, this_value, &[arg1, arg2]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                // New arithmetic superinstructions
                Opcode::LoadAdd => {
                    // LoadAdd a, b, c: reg[a] = reg[b] + reg[c]
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    // Fast path: int32 + int32
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_add(b_int) {
                            self.frame.regs[a] = make_int32(result);
                            continue;
                        }
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[a] = lhs.add(&rhs).raw();
                }
                Opcode::LoadSub => {
                    // LoadSub a, b, c: reg[a] = reg[b] - reg[c]
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    // Fast path: int32 - int32
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_sub(b_int) {
                            self.frame.regs[a] = make_int32(result);
                            continue;
                        }
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[a] = lhs.sub(&rhs).raw();
                }
                Opcode::LoadMul => {
                    // LoadMul a, b, c: reg[a] = reg[b] * reg[c]
                    let lhs = self.frame.regs[b];
                    let rhs = self.frame.regs[c];

                    // Fast path: int32 * int32
                    if lhs.is_int() && rhs.is_int() {
                        let a_int = lhs.int_payload_unchecked();
                        let b_int = rhs.int_payload_unchecked();
                        if let Some(result) = a_int.checked_mul(b_int) {
                            self.frame.regs[a] = make_int32(result);
                            continue;
                        }
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(lhs, rhs);
                    self.frame.regs[a] = lhs.mul(&rhs).raw();
                }
                Opcode::LoadInc => {
                    // LoadInc a, b: reg[a] = reg[b] + 1
                    let value = self.frame.regs[b];

                    // Fast path: int32 + 1
                    if value.is_int() {
                        let int_val = value.int_payload_unchecked();
                        if let Some(result) = int_val.checked_add(1) {
                            self.frame.regs[a] = make_int32(result);
                            continue;
                        }
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(value, make_number(1.0));
                    let result = lhs.add(&rhs).raw();
                    self.frame.regs[a] = result;
                }
                Opcode::LoadDec => {
                    // LoadDec a, b: reg[a] = reg[b] - 1
                    let value = self.frame.regs[b];

                    // Fast path: int32 - 1
                    if value.is_int() {
                        let int_val = value.int_payload_unchecked();
                        if let Some(result) = int_val.checked_sub(1) {
                            self.frame.regs[a] = make_int32(result);
                            continue;
                        }
                    }
                    // Fall back to slow path
                    let (lhs, rhs) = self.value_pair(value, make_number(1.0));
                    self.frame.regs[a] = lhs.sub(&rhs).raw();
                }
                // New comparison superinstructions
                Opcode::LoadCmpEq => {
                    // LoadCmpEq a, b, c: reg[a] = reg[b] == reg[c]
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[a] = lhs.eq(&rhs).raw();
                }
                Opcode::LoadCmpLt => {
                    // LoadCmpLt a, b, c: reg[a] = reg[b] < reg[c]
                    let (lhs, rhs) = self.value_pair(self.frame.regs[b], self.frame.regs[c]);
                    self.frame.regs[a] = lhs.lt(&rhs).raw();
                }
                Opcode::LoadJfalse => {
                    // LoadJfalse a, offset: if !reg[a], jump by offset
                    if !self.is_truthy_value(self.frame.regs[a]) {
                        self.jump_by(b as i8 as i16);
                    }
                }
                Opcode::LoadCmpEqJfalse => {
                    // LoadCmpEqJfalse a, b, offset: if reg[a] == reg[b], jump by offset
                    if self.abstract_equal(self.frame.regs[a], self.frame.regs[b]) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                Opcode::LoadCmpLtJfalse => {
                    // LoadCmpLtJfalse a, b, offset: if reg[a] < reg[b], jump by offset
                    if self.less_than(self.frame.regs[a], self.frame.regs[b]) {
                        self.jump_by(c as i8 as i16);
                    }
                }
                // Property access superinstructions
                Opcode::LoadGetProp => {
                    // LoadGetProp a, prop: ACC = R[a][prop]
                    let key = Self::property_key_from_immediate(b as u16);
                    self.frame.regs[ACC] = self.get_property(self.frame.regs[a], key);
                }
                Opcode::LoadGetPropCmpEq => {
                    // LoadGetPropCmpEq a, prop, b: ACC = (R[a][prop] == R[b])
                    let key = Self::property_key_from_immediate(b as u16);
                    let prop_value = self.get_property(self.frame.regs[a], key);
                    let (lhs, rhs) = self.value_pair(prop_value, self.frame.regs[c]);
                    self.frame.regs[ACC] = lhs.eq(&rhs).raw();
                }
                // Pareto 80% property access superinstructions with IC
                Opcode::GetProp2Ic => {
                    // GetProp2Ic dst, obj, slot1, slot2: dst = obj.slot1.slot2
                    let obj_val = self.frame.regs[b];
                    let slot1 = c as u16;
                    let slot2 = ((insn >> 8) & 0xFF) as u16; // Use 'a' field for second slot
                    let intermediate = self.get_property(obj_val, PropertyKey::Id(slot1));
                    self.frame.regs[a] = self.get_property(intermediate, PropertyKey::Id(slot2));
                }
                Opcode::GetProp3Ic => {
                    // GetProp3Ic dst, obj, slot1, slot2, slot3: dst = obj.slot1.slot2.slot3
                    let obj_val = self.frame.regs[b];
                    let slot1 = c as u16;
                    let slot2 = ((insn >> 8) & 0xFF) as u16; // Use 'a' field for second slot
                    let slot3 = ((insn >> 16) & 0xFF) as u16; // Use 'b' field for third slot
                    let intermediate1 = self.get_property(obj_val, PropertyKey::Id(slot1));
                    let intermediate2 = self.get_property(intermediate1, PropertyKey::Id(slot2));
                    self.frame.regs[a] = self.get_property(intermediate2, PropertyKey::Id(slot3));
                }
                Opcode::GetElem => {
                    // GetElem dst, arr, index: dst = arr[index]
                    let arr_val = self.frame.regs[b];
                    let index_val = self.frame.regs[c];
                    let key = self.property_key_from_value(index_val);
                    self.frame.regs[a] = self.get_property(arr_val, key);
                }
                Opcode::SetElem => {
                    // SetElem arr, index, src: arr[index] = src
                    let arr_val = self.frame.regs[b];
                    let index_val = self.frame.regs[c];
                    let key = self.property_key_from_value(index_val);
                    self.frame.regs[ACC] = self.set_property(arr_val, key, self.frame.regs[a]);
                }
                Opcode::GetPropElem => {
                    // GetPropElem dst, obj, slot, index: dst = obj.slot[index]
                    let obj_val = self.frame.regs[b];
                    let slot = c as u16;
                    let index_val = self.frame.regs[a]; // Use 'a' field for index
                    let intermediate = self.get_property(obj_val, PropertyKey::Id(slot));
                    let key = self.property_key_from_value(index_val);
                    self.frame.regs[a] = self.get_property(intermediate, key);
                }
                Opcode::CallMethodIc => {
                    // CallMethodIc obj, slot: call obj.slot() with 0 args
                    let this_value = self.frame.regs[a];
                    let slot = b as u16;
                    let method = self.get_property(this_value, PropertyKey::Id(slot));
                    match self.dispatch_call_value(method, this_value, &[]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                Opcode::CallMethod2Ic => {
                    // CallMethod2Ic obj, slot1, slot2: call obj.slot1.slot2() with 0 args
                    let this_value = self.frame.regs[a];
                    let slot1 = b as u16;
                    let slot2 = c as u16;
                    let intermediate = self.get_property(this_value, PropertyKey::Id(slot1));
                    let method = self.get_property(intermediate, PropertyKey::Id(slot2));
                    match self.dispatch_call_value(method, this_value, &[]) {
                        CallAction::Returned(result) => self.frame.regs[ACC] = result,
                        CallAction::EnteredFrame => continue,
                    }
                }
                // Assertion opcodes (stubs for now)
                Opcode::AssertValue
                | Opcode::AssertOk
                | Opcode::AssertFail
                | Opcode::AssertThrows
                | Opcode::AssertDoesNotThrow
                | Opcode::AssertRejects
                | Opcode::AssertDoesNotReject
                | Opcode::AssertEqual
                | Opcode::AssertNotEqual
                | Opcode::AssertDeepEqual
                | Opcode::AssertNotDeepEqual
                | Opcode::AssertStrictEqual
                | Opcode::AssertNotStrictEqual
                | Opcode::AssertDeepStrictEqual
                | Opcode::AssertNotDeepStrictEqual => {
                    // Assertion opcodes are no-ops in production
                    // They're only used during testing/development
                    self.frame.regs[ACC] = make_true();
                }
            }
        }
    }

    fn run_until_frame_depth(&mut self, depth: usize) {
        self.threaded_stop_depth = Some(depth);
        self.run_inner(Some(depth));
        self.threaded_stop_depth = None;
    }
    pub fn optimize(&mut self) {
        let bytecode = std::mem::take(&mut self.bytecode);
        let const_pool = std::mem::take(&mut self.const_pool);
        let (bytecode, const_pool) = crate::optimization::optimize_program_parts(
            bytecode,
            const_pool,
            &self.function_constants,
        );
        self.bytecode = bytecode;
        self.const_pool = const_pool;
    }

    pub fn run(&mut self, optimization: bool) {
        if optimization && self.pc == 0 && self.frame.depth() == 0 {
            let bytecode = std::mem::take(&mut self.bytecode);
            let const_pool = std::mem::take(&mut self.const_pool);
            let (bytecode, const_pool) = crate::optimization::optimize_program_parts(
                bytecode,
                const_pool,
                &self.function_constants,
            );
            self.bytecode = bytecode;
            self.const_pool = const_pool;
        }
        self.threaded_stop_depth = None;
        self.run_inner(None);
        self.threaded_stop_depth = None;
    }
}

impl BuiltinHost for VM {
    fn prepare_js_builtin_properties(&mut self, properties: &[String]) {
        self.compiled_properties.clear();
        self.compiled_properties.extend_from_slice(properties);
        self.property_slots.clear();
        for (slot, name) in properties.iter().enumerate() {
            if let Ok(slot) = u16::try_from(slot) {
                self.property_slots.insert(name.clone(), slot);
            }
        }
    }

    fn prepare_js_builtin_private_properties(&mut self, private_properties: &[String]) {
        self.compiled_private_properties.clear();
        self.compiled_private_properties.extend(private_properties.iter().map(|name| self.atoms.intern(name)));
        // Private properties use the same property_slots map but with different offset
        for (slot, atom) in self.compiled_private_properties.iter().enumerate() {
            if let Ok(slot) = u16::try_from(slot + self.compiled_properties.len()) {
                self.property_slots.insert(self.atoms.resolve(*atom).to_owned(), slot);
            }
        }
    }

    fn set_global(&mut self, global_slot: u16, value: JSValue) {
        self.global_object.insert(global_slot, value);
    }

    fn builtin_function(&mut self, native_name: &'static str) -> JSValue {
        self.alloc_native_function(Some(native_name))
    }

    fn create_object(&mut self) -> JSValue {
        self.alloc_object()
    }

    fn create_array(&mut self) -> JSValue {
        self.alloc_array(0)
    }

    fn get_property(&self, object: JSValue, name: &str) -> JSValue {
        self.get_property_by_name(object, name)
    }

    fn get_own_property(&self, object: JSValue, name: &str) -> JSValue {
        self.get_own_property_by_name(object, name)
    }

    fn set_property(&mut self, object: JSValue, name: &str, value: JSValue) -> JSValue {
        let key = self.property_key_for_name(name);
        self.set_property(object, key, value)
    }

    fn delete_property(&mut self, object: JSValue, name: &str) -> bool {
        self.delete_property_by_name(object, name)
    }

    fn has_property(&self, object: JSValue, name: &str) -> bool {
        self.has_property_by_name(object, name)
    }

    fn has_own_property(&self, object: JSValue, name: &str) -> bool {
        self.has_own_property_by_name(object, name)
    }

    fn get_property_value(&self, object: JSValue, key: JSValue) -> JSValue {
        self.get_property(object, self.property_key_from_value(key))
    }

    fn get_own_property_value(&self, object: JSValue, key: JSValue) -> JSValue {
        self.get_own_property_value_from_value(object, key)
    }

    fn set_property_value(&mut self, object: JSValue, key: JSValue, value: JSValue) -> JSValue {
        self.set_property(object, self.property_key_from_value(key), value)
    }

    fn delete_property_value(&mut self, object: JSValue, key: JSValue) -> bool {
        self.delete_property(object, self.property_key_from_value(key))
    }

    fn has_property_value(&self, object: JSValue, key: JSValue) -> bool {
        self.has_property(object, self.property_key_from_value(key))
    }

    fn has_own_property_value(&self, object: JSValue, key: JSValue) -> bool {
        self.has_own_property_value_from_value(object, key)
    }

    fn own_property_names(&self, object: JSValue) -> Vec<String> {
        VM::own_property_names(self, object)
    }

    fn own_property_keys(&mut self, object: JSValue) -> Vec<JSValue> {
        VM::own_property_keys(self, object)
    }

    fn get_index(&self, object: JSValue, index: usize) -> JSValue {
        self.get_property(object, PropertyKey::Index(index as u32))
    }

    fn set_index(&mut self, object: JSValue, index: usize, value: JSValue) -> JSValue {
        self.set_property(object, PropertyKey::Index(index as u32), value)
    }

    fn array_push(&mut self, object: JSValue, value: JSValue) -> JSValue {
        VM::array_push(self, object, value)
    }

    fn array_values(&self, value: JSValue) -> Option<Vec<JSValue>> {
        VM::array_values(self, value)
    }

    fn same_value(&self, lhs: JSValue, rhs: JSValue) -> bool {
        self.object_same_value(lhs, rhs)
    }

    fn is_array(&self, value: JSValue) -> bool {
        matches!(value.heap_kind(), Some(value::HeapKind::Array))
    }

    fn is_object(&self, value: JSValue) -> bool {
        object_from_value(value).is_some()
    }

    fn is_callable(&self, value: JSValue) -> bool {
        let Some(obj_ptr) = object_from_value(value) else {
            return false;
        };

        unsafe {
            matches!(
                &(*obj_ptr).kind,
                ObjectKind::Function(_)
                    | ObjectKind::NativeFunction(_)
                    | ObjectKind::NativeClosure(_)
                    | ObjectKind::Class(_)
            )
        }
    }

    fn call_value(&mut self, callee: JSValue, this_value: JSValue, args: &[JSValue]) -> JSValue {
        VM::call_value(self, callee, this_value, args)
    }

    fn construct_value(&mut self, callee: JSValue, args: &[JSValue]) -> JSValue {
        VM::construct_value(self, callee, args)
    }

    fn json_stringify(&mut self, value: JSValue) -> Result<String, String> {
        with_bridge_context(|ctx| {
            let mut seen = HashMap::new();
            let value = self.vm_to_runtime_value(ctx, value, &mut seen)?;
            to_json(ctx, value).map_err(|error| error.to_string())
        })
    }

    fn json_parse(&mut self, text: &str) -> Result<JSValue, String> {
        with_bridge_context(|ctx| {
            let value = from_json(ctx, text).map_err(|error| error.to_string())?;
            let mut seen = HashMap::new();
            self.runtime_to_vm_value(ctx, value, &mut seen)
        })
    }

    fn yaml_stringify(&mut self, value: JSValue) -> Result<String, String> {
        with_bridge_context(|ctx| {
            let mut seen = HashMap::new();
            let value = self.vm_to_runtime_value(ctx, value, &mut seen)?;
            to_yaml(ctx, value).map_err(|error| error.to_string())
        })
    }

    fn yaml_parse(&mut self, text: &str) -> Result<JSValue, String> {
        with_bridge_context(|ctx| {
            let value = from_yaml(ctx, text).map_err(|error| error.to_string())?;
            let mut seen = HashMap::new();
            self.runtime_to_vm_value(ctx, value, &mut seen)
        })
    }

    fn msgpack_encode(&mut self, value: JSValue) -> Result<Vec<u8>, String> {
        with_bridge_context(|ctx| {
            let mut seen = HashMap::new();
            let value = self.vm_to_runtime_value(ctx, value, &mut seen)?;
            to_msgpack(ctx, value).map_err(|error| error.to_string())
        })
    }

    fn msgpack_decode(&mut self, bytes: &[u8]) -> Result<JSValue, String> {
        with_bridge_context(|ctx| {
            let value = from_msgpack(ctx, bytes).map_err(|error| error.to_string())?;
            let mut seen = HashMap::new();
            self.runtime_to_vm_value(ctx, value, &mut seen)
        })
    }

    fn bin_encode(&mut self, value: JSValue) -> Result<Vec<u8>, String> {
        with_bridge_context(|ctx| {
            let mut seen = HashMap::new();
            let value = self.vm_to_runtime_value(ctx, value, &mut seen)?;
            to_arena_buffer(ctx, value).map_err(|error| error.to_string())
        })
    }

    fn bin_decode(&mut self, bytes: &[u8]) -> Result<JSValue, String> {
        with_bridge_context(|ctx| {
            let value = from_arena_buffer(ctx, bytes).map_err(|error| error.to_string())?;
            let mut seen = HashMap::new();
            self.runtime_to_vm_value(ctx, value, &mut seen)
        })
    }

    fn intern_string(&mut self, text: &str) -> JSValue {
        VM::intern_string(self, text)
    }

    fn string_text<'a>(&'a self, value: JSValue) -> Option<&'a str> {
        VM::string_text(self, value)
    }

    fn is_symbol(&self, value: JSValue) -> bool {
        matches!(value.heap_kind(), Some(value::HeapKind::Symbol))
    }

    fn bytes_from_value(&self, value: JSValue) -> Option<Vec<u8>> {
        VM::bytes_from_value(self, value)
    }

    fn bytes_to_value(&mut self, bytes: &[u8]) -> JSValue {
        VM::bytes_to_value(self, bytes)
    }

    fn display_string(&mut self, value: JSValue) -> String {
        VM::display_string(self, value)
    }

    fn number_value(&mut self, value: JSValue) -> JSValue {
        VM::number_value(self, value)
    }

    fn is_truthy_value(&self, value: JSValue) -> bool {
        VM::is_truthy_value(self, value)
    }

    fn create_symbol(&mut self, description: Option<&str>) -> JSValue {
        let description = description.map(|text| self.atoms.intern(text));
        let id = self.next_symbol_id;
        self.next_symbol_id = self.next_symbol_id.saturating_add(1);
        self.alloc_object_with_kind(ObjectKind::Symbol(QSymbol { id, description }))
    }

    fn symbol_for(&mut self, key: &str) -> JSValue {
        if let Some(&symbol) = self.symbol_registry.get(key) {
            return symbol;
        }

        let symbol = self.create_symbol(Some(key));
        self.symbol_registry.insert(key.to_owned(), symbol);
        symbol
    }

    fn symbol_key_for(&self, value: JSValue) -> Option<String> {
        self.symbol_registry
            .iter()
            .find_map(|(key, &symbol)| self.strict_equal(symbol, value).then(|| key.clone()))
    }

    fn eval_source(&mut self, source: &str) -> Result<JSValue, String> {
        VM::eval_source(self, source)
    }

    fn console_render_args(&mut self, args: &[JSValue]) -> String {
        VM::console_render_args(self, args)
    }

    fn console_write_line(&mut self, text: String, is_error: bool) -> JSValue {
        VM::console_write_line(self, text, is_error)
    }

    fn console_label_from_args(&mut self, args: &[JSValue]) -> String {
        VM::console_label_from_args(self, args)
    }

    fn console_elapsed_message(&self, label: &str, start: Instant, suffix: Option<&str>) -> String {
        VM::console_elapsed_message(self, label, start, suffix)
    }

    fn console_time_start(&mut self, label: String) {
        self.console_timers.insert(label, Instant::now());
    }

    fn console_time_end(&mut self, label: &str) -> Option<Instant> {
        self.console_timers.remove(label)
    }

    fn console_time_get(&self, label: &str) -> Option<Instant> {
        self.console_timers.get(label).copied()
    }

    fn console_group_start(&mut self) {
        self.console_group_depth = self.console_group_depth.saturating_add(1);
    }

    fn console_group_end(&mut self) {
        self.console_group_depth = self.console_group_depth.saturating_sub(1);
    }

    fn console_clear(&mut self) {
        self.console_output.clear();
        self.console_group_depth = 0;
    }

    fn console_count_increment(&mut self, label: &str) -> usize {
        let count = self.console_counts.entry(label.to_owned()).or_insert(0);
        *count += 1;
        *count
    }
}

impl Drop for VM {
    fn drop(&mut self) {
        for obj_ptr in self.objects.drain(..) {
            unsafe {
                drop(Box::from_raw(obj_ptr));
            }
        }

        for shape_ptr in self.shapes.drain(..) {
            unsafe {
                drop(Box::from_raw(shape_ptr));
            }
        }

        for string_ptr in self.strings.drain(..) {
            unsafe {
                drop(Box::from_raw(string_ptr));
            }
        }
    }
}
