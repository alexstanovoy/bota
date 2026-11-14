//! Immutable engine vectors and checked conversion to protocol coordinates.

#![allow(
    clippy::float_arithmetic,
    reason = "the script API uses finite binary64 coordinates"
)]

use std::rc::Rc;

use bota_proto::{Fixed, Vec2};
use luagee::{Args, Lua, LuaError, TableId, Value, Vm};

use crate::BindingError;

#[derive(Clone, Copy)]
struct Vector([f64; 3]);

#[derive(Clone, Copy)]
pub(crate) struct Vectors {
    metatable: TableId,
}

pub(crate) fn number(vm: &mut Vm, args: Args, index: usize) -> Result<f64, LuaError> {
    let number = vm.check_number(args, index)?;
    if !number.is_finite() {
        return Err(vm.arg_error(args, index, "finite number expected"));
    }
    Ok(number)
}

pub(crate) fn read(vm: &mut Vm, value: Value) -> Result<[f64; 3], LuaError> {
    vm.userdata_ref::<Vector>(value)
        .map(|vector| vector.0)
        .ok_or_else(|| vm.error("Vector expected"))
}

impl Vectors {
    pub fn value(self, vm: &mut Vm, value: [f64; 3]) -> Result<Value, LuaError> {
        if value.iter().any(|number| !number.is_finite()) {
            return Err(vm.error("Vector components must be finite"));
        }
        vm.userdata(Vector(value), Some(self.metatable), 0)
    }

    pub fn location(self, vm: &mut Vm, point: Vec2) -> Result<Value, LuaError> {
        self.value(
            vm,
            [
                f64::from(point.x.raw) / 65536.0 - 9216.0,
                f64::from(point.y.raw) / 65536.0 - 9216.0,
                0.0,
            ],
        )
    }
}

pub(crate) fn position(vm: &mut Vm, value: Value) -> Result<Vec2, LuaError> {
    let [x, y, _] = read(vm, value)?;
    let convert = |number: f64| {
        let raw = ((number + 9216.0) * 65536.0).round_ties_even();
        (raw >= f64::from(i32::MIN) && raw <= f64::from(i32::MAX))
            .then_some(Fixed { raw: raw as i32 })
    };
    match (convert(x), convert(y)) {
        (Some(x), Some(y)) => Ok(Vec2 { x, y }),
        _ => Err(vm.error("Vector position is outside protocol coordinates")),
    }
}

fn field(
    lua: &mut Lua,
    table: TableId,
    name: &'static str,
    function: luagee::HostFunction,
) -> Result<(), BindingError> {
    let function = lua.function(name, crate::statistics::instrument("other", function))?;
    let key = lua.string(name.as_bytes())?;
    lua.vm().raw_set(table, key, function)?;
    Ok(())
}

pub(crate) fn install(lua: &mut Lua) -> Result<Vectors, BindingError> {
    let metatable = lua.vm().new_table()?;
    lua.pin(Value::Table(metatable))?;
    let vectors = Vectors { metatable };
    let methods = lua.vm().new_table()?;
    lua.pin(Value::Table(methods))?;
    let constructor = lua.function(
        "Vector",
        crate::statistics::instrument(
            "Vector",
            Rc::new(move |vm, args| {
                let value = [
                    number(vm, args, 0)?,
                    number(vm, args, 1)?,
                    if vm.arg(args, 2) == Value::Nil {
                        0.0
                    } else {
                        number(vm, args, 2)?
                    },
                ];
                let value = vectors.value(vm, value)?;
                vm.results(&[value])
            }),
        ),
    )?;
    lua.set_global("Vector", constructor)?;
    field(
        lua,
        metatable,
        "__index",
        Rc::new(move |vm, args| {
            let [x, y, z] = read(vm, vm.arg(args, 0))?;
            let key = vm.arg(args, 1);
            let value = match key {
                Value::Number(1.0) => Value::Number(x),
                Value::Number(2.0) => Value::Number(y),
                Value::Number(3.0) => Value::Number(z),
                _ => match vm.bytes(key) {
                    Some(b"x") => Value::Number(x),
                    Some(b"y") => Value::Number(y),
                    Some(b"z") => Value::Number(z),
                    _ => vm.raw_get(methods, key)?,
                },
            };
            vm.results(&[value])
        }),
    )?;
    for (name, operation) in [("__add", 0), ("__sub", 1), ("__mul", 2), ("__div", 3)] {
        field(
            lua,
            metatable,
            name,
            Rc::new(move |vm, args| {
                let left = vm.arg(args, 0);
                let right = vm.arg(args, 1);
                let unpack = |vm: &mut Vm, value| match value {
                    Value::Number(n) if n.is_finite() => Ok([n; 3]),
                    _ => read(vm, value),
                };
                let (a, b) = match operation {
                    0 | 1 => (read(vm, left)?, read(vm, right)?),
                    2 => (unpack(vm, left)?, unpack(vm, right)?),
                    _ => {
                        let left = read(vm, left)?;
                        let right = number(vm, args, 1)?;
                        (left, [right; 3])
                    }
                };
                let value = std::array::from_fn(|i| match operation {
                    0 => a[i] + b[i],
                    1 => a[i] - b[i],
                    2 => a[i] * b[i],
                    _ => a[i] / b[i],
                });
                let value = vectors.value(vm, value)?;
                vm.results(&[value])
            }),
        )?;
    }
    field(
        lua,
        metatable,
        "__unm",
        Rc::new(move |vm, args| {
            let a = read(vm, vm.arg(args, 0))?;
            let value = vectors.value(vm, a.map(|n| -n))?;
            vm.results(&[value])
        }),
    )?;
    field(
        lua,
        metatable,
        "__eq",
        Rc::new(|vm, args| {
            let a = read(vm, vm.arg(args, 0))?;
            let b = read(vm, vm.arg(args, 1))?;
            vm.results(&[Value::Boolean(a == b)])
        }),
    )?;
    field(
        lua,
        metatable,
        "__tostring",
        Rc::new(|vm, args| {
            let [x, y, z] = read(vm, vm.arg(args, 0))?;
            let value = vm.new_string(format!("Vector [{x} {y} {z}]").as_bytes())?;
            vm.results(&[value])
        }),
    )?;
    for (name, operation) in [
        ("Length", 0),
        ("Length2D", 1),
        ("Normalized", 2),
        ("Dot", 3),
        ("Cross", 4),
        ("Lerp", 5),
    ] {
        field(
            lua,
            methods,
            name,
            Rc::new(move |vm, args| {
                let a = read(vm, vm.arg(args, 0))?;
                let length = a[0]
                    .hypot(a[1])
                    .hypot(if operation == 1 { 0.0 } else { a[2] });
                let value = match operation {
                    0 | 1 => Value::Number(length),
                    2 => {
                        let scale = a.into_iter().map(f64::abs).fold(0.0, f64::max);
                        let normalized = if scale == 0.0 {
                            [0.0; 3]
                        } else {
                            let scaled = a.map(|number| number / scale);
                            let length = scaled[0].hypot(scaled[1]).hypot(scaled[2]);
                            scaled.map(|number| number / length)
                        };
                        vectors.value(vm, normalized)?
                    }
                    _ => {
                        let b = read(vm, vm.arg(args, 1))?;
                        match operation {
                            3 => Value::Number(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]),
                            4 => vectors.value(
                                vm,
                                [
                                    a[1] * b[2] - a[2] * b[1],
                                    a[2] * b[0] - a[0] * b[2],
                                    a[0] * b[1] - a[1] * b[0],
                                ],
                            )?,
                            _ => {
                                let t = number(vm, args, 2)?;
                                vectors
                                    .value(vm, std::array::from_fn(|i| a[i] + (b[i] - a[i]) * t))?
                            }
                        }
                    }
                };
                if matches!(value, Value::Number(n) if !n.is_finite()) {
                    return Err(vm.error("Vector result is not finite"));
                }
                vm.results(&[value])
            }),
        )?;
    }
    let key = lua.string(b"__metatable")?;
    lua.vm().raw_set(metatable, key, Value::Boolean(false))?;
    Ok(vectors)
}
