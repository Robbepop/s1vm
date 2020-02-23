
use crate::*;
use crate::function::*;
use crate::error::*;

#[derive(Debug, Clone, Copy)]
enum BlockKind {
  Block,
  Loop,
  If,
  Else,
}

#[derive(Debug, Clone)]
pub enum Action {
  Return(Option<StackValue>),
  End,
  Branch(u32),
}

type EvalFunc = Box<dyn Fn(&vm::State, &mut Store) -> Trap<Action>>;

type OpFunc = Box<dyn Fn(&vm::State, &mut Store) -> Trap<StackValue>>;

enum Input {
  Local(u32),
  Const(StackValue),
  Op(OpFunc),
}

impl Input {
  pub fn resolv(&self, state: &vm::State, store: &mut Store) -> Trap<StackValue> {
    match self {
      Input::Local(local_idx) => {
        //eprintln!("-- run compiled: GetLocal({})", local_idx);
        Ok(store.stack.get_local_val(*local_idx))
      },
      Input::Const(const_val) => {
        //eprintln!("-- run compiled: Const({:?})", const_val);
        Ok(*const_val)
      },
      Input::Op(closure) => closure(state, store),
    }
  }
}

struct Block
{
  kind: BlockKind,
  depth: u32,
  eval: Vec<EvalFunc>,
}

impl Block {
  pub fn new(kind: BlockKind, depth: u32) -> Block {
    Block {
      kind,
      depth,
      eval: vec![],
    }
  }

  pub fn depth(&self) -> u32 {
    self.depth
  }

  pub fn push(&mut self, f: EvalFunc) {
    self.eval.push(f);
  }

  pub fn run(&self, state: &vm::State, store: &mut Store) -> Trap<Action> {
    //eprintln!("---- run block: {:?}", self.kind);
    for f in self.eval.iter() {
      let ret = f(state, store)?;
      //eprintln!("---- evaled: ret = {:?}", ret);
      match ret {
        Action::Return(_) => {
          // Keep passing return value up, until we get to the function block.
          return Ok(ret);
        },
        Action::End => {
          // sub-block finished, continue this block.
          continue;
        },
        Action::Branch(depth) => {
          if self.depth > depth {
            // keep passing action lower.
            return Ok(ret);
          } else if self.depth == depth {
            // handle Branch here.
            todo!("handle branch")
          } else {
            unreachable!("Can't branch into a sub-block.");
          }
        }
      }
    }
    Ok(Action::End)
  }
}

pub struct State {
  values: Vec<Input>,
  pub depth: u32,
  pub pc: usize,
}

impl State {
  pub fn new() -> State {
    State {
      values: vec![],
      depth: 0,
      pc: 0,
    }
  }

  fn pop(&mut self) -> Result<Input> {
    self.values.pop()
      .ok_or_else(|| {
        Error::ValidationError(format!("Value stack empty"))
      })
  }

  fn push(&mut self, input: Input) {
    self.values.push(input);
  }
}

pub struct Compiler {
  module: bwasm::Module,
  compiled: Vec<Function>,

  func_idx: u32,
  ret_type: Option<ValueType>,
  code: Vec<bwasm::Instruction>,
  pc_end: usize,
}

impl Compiler {
  pub fn new(module: &bwasm::Module) -> Compiler {
    Compiler {
      module: module.clone(),
      compiled: vec![],

      func_idx: 0,
      ret_type: None,
      code: vec![],
      pc_end: 0,
    }
  }

  pub fn compile(mut self) -> Result<Vec<Function>> {
    let len = self.module.functions().len() as u32;
    for idx in 0..len {
      self.compile_function(idx)?;
    }
    Ok(self.compiled)
  }

  fn compile_function(&mut self, func_idx: u32) -> Result<()> {
    self.func_idx = func_idx;
    let func = self.module.get_func(func_idx)
      .ok_or(Error::FuncNotFound)?;

    // Compile function into a closure
    self.code = func.instructions().to_vec();
    self.ret_type = func.return_type().map(ValueType::from);
    self.pc_end = self.code.len();

    /*
    eprintln!("----- Compiling function: {}", func.name());
    for (pc, op) in self.code.iter().enumerate() {
      eprintln!("- {}: {:?}", pc, op);
    }
    // */

    let mut state = State::new();
    let block = self.compile_block(&mut state, BlockKind::Block)?;

    self.compiled.push(Function::new_compiled(func,
    Box::new(move |state: &vm::State, store: &mut Store| -> Trap<Option<StackValue>>
    {
      match block.run(state, store)? {
        Action::Return(ret_value) => {
          //eprintln!("--- Function return: {:?}", ret_value);
          return Ok(ret_value);
        },
        _ => {
          unreachable!("Compiled function missing 'Return' action.");
        },
      }
    })));

    //eprintln!("---------- depth = {}, values = {}", state.depth, state.len());
    Ok(())
  }

  fn compile_block(&self, state: &mut State, kind: BlockKind) -> Result<Block> {
    let mut block = Block::new(kind, state.depth);
    //eprintln!("compile block: depth: {} {:?}", block.depth(), kind);
    state.depth += 1;
    if state.depth > 4 {
      panic!("compile overflow");
    }
    // compile function opcodes.
    loop {
      use parity_wasm::elements::Instruction::*;
      if state.pc > self.pc_end {
        break;
      }
      let pc = state.pc;
      let op = &self.code[pc];
      //eprintln!("compile {}: {:?}", pc, op);
      match op {
  	    Block(_) => {
          state.pc += 1;
          self.compile_block(state, BlockKind::Block)?;
        },
  	    Loop(_) => {
          state.pc += 1;
          self.compile_loop(state)?;
        },
  	    If(_) => {
          state.pc += 1;
          self.compile_if(&mut block, state)?;
        },
  	    Else => {
          match kind {
            BlockKind::If => {
              break;
            },
            _ => {
              return Err(Error::ValidationError(format!("invalid 'else' block, missing 'if'")));
            },
          }
        },
  	    End => {
          if block.depth() == 0 {
            //self.emit_return(state, &mut block)?;
          }
          break;
        },
  	    Return => {
          self.emit_return(state, &mut block)?;
        },
  	    Br(_block_depth) => {
          todo!("");
        },
  	    BrIf(_block_depth) => {
          todo!("");
        },
  	    BrTable(ref _br_table) => {
          todo!("");
        },

        Call(func_idx) => {
          let idx = *func_idx;
          let val = state.pop()?;
          state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
        //eprintln!("--- run compiled: {} Call({})", pc, idx);
            let val = val.resolv(state, store)?;
            store.stack.push_val(val)?;
            if let Some(ret) = state.invoke_function(store, idx)? {
              Ok(ret)
            } else {
              Ok(StackValue(0))
            }
          })));
        },

	      GetLocal(local_idx) => {
          state.push(Input::Local(*local_idx));
        },
	      I32Const(val) => {
          state.push(Input::Const(StackValue(*val as _)));
        },
	      I64Const(val) => {
          state.push(Input::Const(StackValue(*val as _)));
        },

	      I32Add => {
          i32_ops::add(state)?;
        },
	      I32Sub => {
          i32_ops::sub(state)?;
        },
        I32LtS => {
          i32_ops::lt_s(state)?;
        },
        _ => todo!("implment opcode"),
      };
      state.pc += 1;
    }

    state.depth -= 1;
    //eprintln!("end block: depth: {} {:?}", block.depth(), kind);
    Ok(block)
  }

  fn emit_return(&self, state: &mut State, block: &mut Block) -> Result<()> {
    if self.ret_type.is_some() {
      let ret = state.pop()?;
      match ret {
        Input::Local(local_idx) => {
          block.push(Box::new(move |_state: &vm::State, store: &mut Store| -> Trap<Action> {
            let ret = store.stack.get_local_val(local_idx);
            Ok(Action::Return(Some(StackValue(ret.0 as _))))
          }));
        },
        Input::Const(const_val) => {
          block.push(Box::new(move |_state: &vm::State, _store: &mut Store| -> Trap<Action> {
            let ret = const_val;
            Ok(Action::Return(Some(StackValue(ret.0 as _))))
          }));
        },
        Input::Op(closure) => {
          block.push(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<Action> {
            let ret = closure(state, store)?;
            Ok(Action::Return(Some(StackValue(ret.0 as _))))
          }));
        },
      }
    } else {
      block.push(Box::new(move |_state: &vm::State, _store: &mut Store| -> Trap<Action> {
        //eprintln!("--- run compiled RETURN: no value");
        Ok(Action::Return(None))
      }));
    }
    Ok(())
  }

  fn compile_loop(&self, state: &mut State) -> Result<Block> {
    self.compile_block(state, BlockKind::Loop)
  }

  fn compile_if(&self, parent: &mut Block, state: &mut State) -> Result<()> {
    // pop condition value.
    let val = state.pop()?;

    // compile 'If' block.
    let if_block = self.compile_block(state, BlockKind::If)?;

    // Check for Else block
    use parity_wasm::elements::Instruction::*;
    let else_block = match &self.code[state.pc] {
      Else => {
        Some(self.compile_else(state)?)
      },
      End => {
        None
      },
      _ => {
        unreachable!("missing end of 'If' block");
      }
    };

    // Build closure.
    if let Some(else_block) = else_block {
      match val {
        Input::Op(closure) => {
          parent.push(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<Action>
          {
            let val = closure(state, store)?;
            if val.0 == 0 {
              else_block.run(state, store)
            } else {
              if_block.run(state, store)
            }
          }));
        },
        _ => {
          parent.push(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<Action>
          {
            let val = val.resolv(state, store)?;
            if val.0 == 0 {
              else_block.run(state, store)
            } else {
              if_block.run(state, store)
            }
          }));
        },
      }
    } else {
      match val {
        Input::Op(closure) => {
          parent.push(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<Action>
          {
            let val = closure(state, store)?;
            if val.0 == 0 {
              Ok(Action::End)
            } else {
              if_block.run(state, store)
            }
          }));
        },
        _ => {
          parent.push(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<Action>
          {
            let val = val.resolv(state, store)?;
            if val.0 == 0 {
              Ok(Action::End)
            } else {
              if_block.run(state, store)
            }
          }));
        },
      }
    }
    Ok(())
  }

  fn compile_else(&self, state: &mut State) -> Result<Block> {
    self.compile_block(state, BlockKind::Else)
  }
}

macro_rules! impl_int_binops {
  ($name: ident, $type: ty, $op: ident) => {
    pub fn $name(state: &mut State) -> Result<()> {
      let right = state.pop()?;
      let left = state.pop()?;
      match left {
        Input::Local(left_idx) => {
          match right {
            Input::Const(right_const) => {
              state.push(Input::Op(Box::new(move |_state: &vm::State, store: &mut Store| -> Trap<StackValue> {
                let left = store.stack.get_local_val(left_idx);
                let right = right_const;
                let res = (left.0 as $type).$op(right.0 as $type);
                Ok(StackValue(res as _))
              })));
              return Ok(());
            },
            _ => (),
          }
        },
        Input::Op(left_closure) => {
          match right {
            Input::Local(right_idx) => {
              state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
                //eprintln!("-------- fast binop: 1 closures");
                let left = left_closure(state, store)?;
                let right = store.stack.get_local_val(right_idx);
                let res = (left.0 as $type).$op(right.0 as $type);
                Ok(StackValue(res as _))
              })));
              return Ok(());
            },
            Input::Const(right_const) => {
              state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
                //eprintln!("-------- fast binop: 1 closures");
                let left = left_closure(state, store)?;
                let right = right_const;
                let res = (left.0 as $type).$op(right.0 as $type);
                Ok(StackValue(res as _))
              })));
              return Ok(());
            },
            Input::Op(right_closure) => {
              state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
                //eprintln!("-------- fast binop: 2 closures");
                let left = left_closure(state, store)?;
                let right = right_closure(state, store)?;
                let res = (left.0 as $type).$op(right.0 as $type);
                Ok(StackValue(res as _))
              })));
              return Ok(());
            },
          }
        },
        _ => (),
      }
      state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
        eprintln!("-------- slow binop.");
        let left = left.resolv(state, store)?;
        let right = right.resolv(state, store)?;
        let res = (left.0 as $type).$op(right.0 as $type);
        Ok(StackValue(res as _))
      })));
      Ok(())
    }
  };
  ($name: ident, $type: ty, $op: ident, $as_type: ty) => {
    pub fn $name(state: &mut State) -> Result<()> {
      let right = state.pop()?;
      let left = state.pop()?;
      state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
        let left = left.resolv(state, store)?;
        let right = right.resolv(state, store)?;
        let res = (left.0 as $type).$op(right.0 as $type) as $as_type;
        Ok(StackValue(res as _))
      })));
      Ok(())
    }
  };
  ($name: ident, $type: ty, $type2: ty, $op: ident, $as_type: ty) => {
    pub fn $name(state: &mut State) -> Result<()> {
      let right = state.pop()?;
      let left = state.pop()?;
      state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
        let left = left.resolv(state, store)?;
        let right = right.resolv(state, store)?;
        let res = (left.0 as $type).$op(right.0 as $type2) as $as_type;
        Ok(StackValue(res as _))
      })));
      Ok(())
    }
  };
  ($name: ident, $type: ty, $op: ident, $as_type: ty, $mask: expr) => {
    pub fn $name(state: &mut State) -> Result<()> {
      let right = state.pop()?;
      let left = state.pop()?;
      state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
        let left = left.resolv(state, store)?;
        let right = right.resolv(state, store)?;
        let right = (right.0 as $type) & $mask;
        let res = (left.0 as $type).$op(right as u32) as $as_type;
        Ok(StackValue(res as _))
      })));
      Ok(())
    }
  };
}

macro_rules! impl_int_binops_div {
  ($name: ident, $type: ty, $op: ident, $as_type: ty) => {
    pub fn $name(store: &mut Store) -> Trap<()> {
      store.stack.binop(|left, right| {
        let res = (left.0 as $type).$op(right.0 as $type)
          .ok_or_else(|| {
            if (right.0 as $type) == 0 {
              TrapKind::DivisionByZero
            } else {
              TrapKind::InvalidConversionToInt
            }
          })?;
        *left = StackValue((res as $as_type) as _);
        Ok(())
      })
    }
  };
}

macro_rules! impl_int_relops {
  ($name: ident, $type: ty, $relop: expr) => {
    pub fn $name(state: &mut State) -> Result<()> {
      let val = state.pop()?;
      state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
        let val = val.resolv(state, store)?;
        let res = $relop(val.0 as $type);
        Ok(StackValue(res as _))
      })));
      Ok(())
    }
  };
  ($name: ident, $type: ty, $type2: ty, $relop: expr) => {
    pub fn $name(state: &mut State) -> Result<()> {
      let right = state.pop()?;
      let left = state.pop()?;
      match left {
        Input::Local(left_idx) => {
          match right {
            Input::Const(right_const) => {
              state.push(Input::Op(Box::new(move |_state: &vm::State, store: &mut Store| -> Trap<StackValue> {
                let left = store.stack.get_local_val(left_idx);
                let right = right_const;
                let res = $relop(left.0 as $type, right.0 as $type2);
                Ok(StackValue(res as _))
              })));
              return Ok(());
            },
            _ => (),
          }
        },
        _ => (),
      }
      state.push(Input::Op(Box::new(move |state: &vm::State, store: &mut Store| -> Trap<StackValue> {
        let left = left.resolv(state, store)?;
        let right = right.resolv(state, store)?;
        let res = $relop(left.0 as $type, right.0 as $type2);
        Ok(StackValue(res as _))
      })));
      Ok(())
    }
  };
}

macro_rules! impl_numeric_ops {
  ($op_mod: ident, $type: ty, $type_u: ty) => {
    #[allow(dead_code)]
    mod $op_mod {
      use std::ops::*;
      use super::*;

      pub fn load(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn load8_s(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn load8_u(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn load16_s(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn load16_u(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn load32_s(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn load32_u(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }

      pub fn store(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn store8(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn store16(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }
      pub fn store32(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }

      pub fn clz(store: &mut Store) -> Trap<()> {
        let val: $type = store.stack.pop()?;
        store.stack.push(val.leading_zeros())
      }
      pub fn ctz(store: &mut Store) -> Trap<()> {
        let val: $type = store.stack.pop()?;
        store.stack.push(val.trailing_zeros())
      }
      pub fn popcnt(store: &mut Store) -> Trap<()> {
        let val: $type = store.stack.pop()?;
        store.stack.push(val.count_ones())
      }

      impl_int_binops!(add, $type, wrapping_add);
      impl_int_binops!(sub, $type, wrapping_sub);

      impl_int_binops!(mul, $type, wrapping_mul);

      impl_int_binops_div!(div_s, $type, checked_div, i64);
      impl_int_binops_div!(div_u, $type, checked_div, u64);
      impl_int_binops_div!(rem_s, $type, checked_rem, i64);
      impl_int_binops_div!(rem_u, $type, checked_rem, u64);

      impl_int_binops!(and, $type, bitand);
      impl_int_binops!(or, $type, bitor);
      impl_int_binops!(xor, $type, bitxor);
      impl_int_binops!(shl, $type, wrapping_shl, $type_u, 0x1F);
      impl_int_binops!(shr_s, $type, wrapping_shr, $type_u, 0x1F);
      impl_int_binops!(shr_u, $type, wrapping_shr, $type_u, 0x1F);
      impl_int_binops!(rotl, $type, u32, rotate_left, u64);
      impl_int_binops!(rotr, $type, u32, rotate_right, u64);

      impl_int_relops!(eqz, $type, |val| {
        val == Default::default()
      });
      impl_int_relops!(eq, $type, $type, |left, right| {
        left == right
      });
      impl_int_relops!(ne, $type, $type, |left, right| {
        left != right
      });
      impl_int_relops!(lt_s, $type, $type, |left, right| {
        left < right
      });
      impl_int_relops!(lt_u, $type_u, $type_u, |left, right| {
        left < right
      });
      impl_int_relops!(gt_s, $type, $type, |left, right| {
        left > right
      });
      impl_int_relops!(gt_u, $type_u, $type_u, |left, right| {
        left > right
      });
      impl_int_relops!(le_s, $type, $type, |left, right| {
        left <= right
      });
      impl_int_relops!(le_u, $type_u, $type_u, |left, right| {
        left <= right
      });
      impl_int_relops!(ge_s, $type, $type, |left, right| {
        left >= right
      });
      impl_int_relops!(ge_u, $type_u, $type_u, |left, right| {
        left >= right
      });

      pub fn trunc_s_f32(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn trunc_u_f32(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn trunc_s_f64(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn trunc_u_f64(_store: &mut Store) -> Trap<()> {
        todo!();
      }

    }
  };
}

impl_numeric_ops!(i32_ops, i32, u32);
impl_numeric_ops!(i64_ops, i64, u64);

macro_rules! impl_float_numeric_ops {
  ($op_mod: ident, $type: ty) => {
    #[allow(dead_code)]
    mod $op_mod {

      use super::*;

      pub fn load(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }

      pub fn store(_store: &mut Store, _offset: u32) -> Trap<()> {
        todo!();
      }

      pub fn abs(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn neg(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn ceil(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn floor(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn trunc(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn nearest(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn sqrt(_store: &mut Store) -> Trap<()> {
        todo!();
      }

      pub fn add(store: &mut Store) -> Trap<()> {
        let (left, right) = store.stack.pop_pair()? as ($type, $type);
        let res = left + right;
        store.stack.push(res)?;
        Ok(())
      }

      pub fn sub(store: &mut Store) -> Trap<()> {
        let (left, right) = store.stack.pop_pair()? as ($type, $type);
        let res = left - right;
        store.stack.push(res)?;
        Ok(())
      }

      pub fn mul(store: &mut Store) -> Trap<()> {
        let (left, right) = store.stack.pop_pair()? as ($type, $type);
        let res = left * right;
        store.stack.push(res)?;
        Ok(())
      }
      pub fn div(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn min(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn max(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn copysign(_store: &mut Store) -> Trap<()> {
        todo!();
      }

      pub fn eq(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn ne(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn lt(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn gt(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn le(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn ge(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn convert_s_i32(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn convert_u_i32(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn convert_s_i64(_store: &mut Store) -> Trap<()> {
        todo!();
      }
      pub fn convert_u_i64(_store: &mut Store) -> Trap<()> {
        todo!();
      }
    }
  };
}

impl_float_numeric_ops!(f32_ops, f32);
impl_float_numeric_ops!(f64_ops, f64);

