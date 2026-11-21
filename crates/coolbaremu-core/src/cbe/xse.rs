//! XSE 0.8 scene script reader and execution state.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Value {
    #[default]
    Zero,
    Integer(i32),
    String(u32),
}

impl Value {
    pub(crate) fn integer(self) -> i32 {
        match self {
            Self::Integer(value) => value,
            _ => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct Operand {
    kind: u8,
    value: i32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Instruction {
    opcode: u16,
    operands: Vec<Operand>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Script {
    code: Vec<Instruction>,
    strings: Vec<Vec<u8>>,
    functions: BTreeMap<String, usize>,
    commands: Vec<String>,
    variables: Vec<Value>,
    stack: Vec<Value>,
    result: Value,
    pc: Option<usize>,
    exit_index: usize,
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        let (value, rest) = self.0.split_at_checked(len)?;
        self.0 = rest;
        Some(value)
    }

    fn byte(&mut self) -> Option<u8> {
        Some(*self.take(1)?.first()?)
    }

    fn word(&mut self) -> Option<u32> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?))
    }

    fn count(&mut self, minimum_size: usize) -> Option<usize> {
        let count = self.word()? as usize;
        (count <= self.0.len() / minimum_size).then_some(count)
    }

    fn name(&mut self) -> Option<String> {
        let length = self.byte()?.into();
        Some(
            std::str::from_utf8(self.take(length)?)
                .ok()?
                .to_ascii_uppercase(),
        )
    }
}

pub(crate) enum Step {
    Continue,
    Host(usize),
    Finished,
}

impl Script {
    pub(crate) fn parse(data: &[u8]) -> Option<Self> {
        let mut reader = Reader(data);
        if reader.take(6)? != b"XSE0\0\x08" {
            return None;
        }
        reader.word()?; // Stack capacity.
        let globals = reader.word()?;
        if globals > 65536 {
            return None;
        }
        reader.take(10)?; // Main flag, main index, and priority.
        let count_instructions = reader.count(3)?;
        let mut code = Vec::with_capacity(count_instructions);
        const EXIT: [u8; 8] = [32, 0, 1, 0, 0, 0, 0, 0];
        // This compiler can emit more physical instructions than the header
        // reports. The final EXIT is followed by the string table.
        while code.len() < count_instructions || !reader.0.starts_with(&EXIT) {
            let opcode = u16::from_le_bytes(reader.take(2)?.try_into().ok()?);
            let count = reader.byte()?;
            let mut operands = Vec::with_capacity(count.into());
            for _ in 0..count {
                let kind = reader.byte()?;
                let value = match kind {
                    0..=7 => reader.word()? as i32,
                    8 => i32::from(reader.byte()?),
                    _ => return None,
                };
                if kind == 4 {
                    reader.word()?;
                }
                operands.push(Operand { kind, value });
            }
            code.push(Instruction { opcode, operands });
        }
        reader.take(EXIT.len())?;
        let count = reader.count(4)?;
        let mut strings = Vec::with_capacity(count);
        for _ in 0..count {
            let len = reader.word()? as usize;
            strings.push(reader.take(len)?.to_vec());
        }
        let count = reader.count(10)?;
        let mut functions = BTreeMap::new();
        for _ in 0..count {
            let boundary = reader.word()? as usize;
            reader.take(5)?; // Parameter and local counts.
            let name = reader.name()?;
            // The initial function starts at zero. Later function indices
            // and branch targets point to the preceding instruction.
            let entry = if boundary == 0 {
                0
            } else {
                boundary.checked_add(1)?
            };
            if entry <= code.len() {
                functions.insert(name, entry);
            }
        }
        let count = reader.count(1)?;
        let commands = (0..count)
            .map(|_| reader.name())
            .collect::<Option<Vec<_>>>()?;
        if !reader.0.iter().all(|byte| *byte == 0) {
            return None;
        }
        Some(Self {
            code,
            strings,
            functions,
            commands,
            variables: vec![Value::Zero; globals as usize],
            stack: Vec::new(),
            result: Value::Zero,
            pc: None,
            exit_index: count_instructions,
        })
    }

    pub(crate) fn start(&mut self, function: &str) {
        self.pc = self.functions.get(function).copied();
        self.stack.clear();
    }

    pub(crate) fn stop(&mut self) {
        self.pc = None;
        self.stack.clear();
    }

    pub(crate) fn is_running(&self) -> bool {
        self.pc.is_some()
    }

    pub(crate) fn command(&self, index: usize) -> Option<&str> {
        self.commands.get(index).map(String::as_str)
    }

    pub(crate) fn parameter(&self, index: usize) -> Option<Value> {
        self.stack.iter().rev().nth(index).copied()
    }

    pub(crate) fn string(&self, index: u32) -> Option<&[u8]> {
        self.strings.get(index as usize).map(Vec::as_slice)
    }

    pub(crate) fn return_from_host(&mut self, count: usize, result: Value) {
        self.stack.truncate(self.stack.len().saturating_sub(count));
        self.result = result;
    }

    fn read(&self, operand: Operand) -> Option<Value> {
        match operand.kind {
            0 => Some(Value::Integer(operand.value)),
            2 => Some(Value::String(operand.value as u32)),
            3 => self.variables.get(operand.value as usize).copied(),
            8 if operand.value == 0 => Some(self.result),
            _ => None,
        }
    }

    fn branch_target(&self, target: Operand) -> Option<usize> {
        let next = (target.value as usize).checked_add(1)?;
        // The compiler uses the declared instruction count for the exit
        // label even when it has emitted extra physical records.
        (next != self.exit_index).then_some(next)
    }

    pub(crate) fn step(&mut self) -> Step {
        let step = self.execute();
        if matches!(step, Step::Finished) {
            self.stop();
        }
        step
    }

    fn execute(&mut self) -> Step {
        let Some(pc) = self.pc else {
            return Step::Finished;
        };
        let Some(Instruction { opcode, operands }) = self.code.get(pc) else {
            return Step::Finished;
        };
        self.pc = Some(pc + 1);
        match (*opcode, operands.as_slice()) {
            (0, []) => {}
            (0, [dst, src]) if dst.kind == 3 => {
                let Some(value) = self.read(*src) else {
                    return Step::Finished;
                };
                let Some(slot) = self.variables.get_mut(dst.value as usize) else {
                    return Step::Finished;
                };
                *slot = value;
            }
            (19, [target]) if target.kind == 5 => {
                self.pc = self.branch_target(*target);
            }
            (20..=25, [left, right, target]) if target.kind == 5 => {
                let (Some(left), Some(right)) = (self.read(*left), self.read(*right)) else {
                    return Step::Finished;
                };
                let (left, right) = (left.integer(), right.integer());
                let taken = match opcode {
                    20 => left == right,
                    21 => left != right,
                    22 => left > right,
                    23 => left < right,
                    24 => left >= right,
                    _ => left <= right,
                };
                if taken {
                    self.pc = self.branch_target(*target);
                }
            }
            (7, [dst]) if dst.kind == 3 => {
                let Some(slot) = self.variables.get_mut(dst.value as usize) else {
                    return Step::Finished;
                };
                *slot = Value::Integer(slot.integer().wrapping_add(1));
            }
            (26, [value]) if self.stack.len() < 256 => {
                let Some(value) = self.read(*value) else {
                    return Step::Finished;
                };
                self.stack.push(value);
            }
            (27, [dst]) if dst.kind == 3 => {
                let Some(slot) = self.variables.get_mut(dst.value as usize) else {
                    return Step::Finished;
                };
                let Some(value) = self.stack.pop() else {
                    return Step::Finished;
                };
                *slot = value;
            }
            (30, [command]) if command.kind == 7 => return Step::Host(command.value as usize),
            _ => return Step::Finished,
        }
        Step::Continue
    }
}
