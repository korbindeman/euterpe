#![no_std]

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamValue {
    Bool(bool),
    Float(f32),
    Choice(u8),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ParamKind {
    Toggle {
        default: bool,
    },
    Knob {
        min: f32,
        max: f32,
        default: f32,
    },
    Choice {
        options: &'static [&'static str],
        default: u8,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    pub id: &'static str,
    pub label: &'static str,
    pub group: &'static str,
    pub kind: ParamKind,
}

impl ParamSpec {
    pub const fn toggle(id: &'static str) -> Self {
        Self {
            id,
            label: id,
            group: "Main",
            kind: ParamKind::Toggle { default: false },
        }
    }

    pub const fn knob(id: &'static str) -> Self {
        Self {
            id,
            label: id,
            group: "Main",
            kind: ParamKind::Knob {
                min: 0.0,
                max: 1.0,
                default: 0.0,
            },
        }
    }

    pub const fn choice(id: &'static str, options: &'static [&'static str]) -> Self {
        Self {
            id,
            label: id,
            group: "Main",
            kind: ParamKind::Choice {
                options,
                default: 0,
            },
        }
    }

    pub const fn label(mut self, label: &'static str) -> Self {
        self.label = label;
        self
    }

    pub const fn group(mut self, group: &'static str) -> Self {
        self.group = group;
        self
    }

    pub const fn range(mut self, min: f32, max: f32) -> Self {
        if let ParamKind::Knob { default, .. } = self.kind {
            self.kind = ParamKind::Knob { min, max, default };
        }
        self
    }

    pub const fn default_bool(mut self, default: bool) -> Self {
        if let ParamKind::Toggle { .. } = self.kind {
            self.kind = ParamKind::Toggle { default };
        }
        self
    }

    pub const fn default_float(mut self, default: f32) -> Self {
        if let ParamKind::Knob { min, max, .. } = self.kind {
            self.kind = ParamKind::Knob { min, max, default };
        }
        self
    }

    pub const fn default_choice(mut self, default: u8) -> Self {
        if let ParamKind::Choice { options, .. } = self.kind {
            self.kind = ParamKind::Choice { options, default };
        }
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareBindingKind {
    SetNormalized,
    SetBool,
    Toggle,
    Cycle,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HardwareControlKind {
    Pot,
    Button,
    Switch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HardwareBinding {
    pub control: &'static str,
    pub control_kind: HardwareControlKind,
    pub param_id: &'static str,
    pub binding_kind: HardwareBindingKind,
    pub page: &'static str,
}

impl HardwareBinding {
    pub const fn pot(control: &'static str) -> Self {
        Self {
            control,
            control_kind: HardwareControlKind::Pot,
            param_id: "",
            binding_kind: HardwareBindingKind::SetNormalized,
            page: "Main",
        }
    }

    pub const fn button(control: &'static str) -> Self {
        Self {
            control,
            control_kind: HardwareControlKind::Button,
            param_id: "",
            binding_kind: HardwareBindingKind::Toggle,
            page: "Main",
        }
    }

    pub const fn switch(control: &'static str) -> Self {
        Self {
            control,
            control_kind: HardwareControlKind::Switch,
            param_id: "",
            binding_kind: HardwareBindingKind::SetBool,
            page: "Main",
        }
    }

    pub const fn bind(mut self, param_id: &'static str) -> Self {
        self.param_id = param_id;
        self
    }

    pub const fn page(mut self, page: &'static str) -> Self {
        self.page = page;
        self
    }

    pub const fn normalized(mut self) -> Self {
        self.binding_kind = HardwareBindingKind::SetNormalized;
        self
    }

    pub const fn set_bool(mut self) -> Self {
        self.binding_kind = HardwareBindingKind::SetBool;
        self
    }

    pub const fn toggle(mut self) -> Self {
        self.binding_kind = HardwareBindingKind::Toggle;
        self
    }

    pub const fn cycle(mut self) -> Self {
        self.binding_kind = HardwareBindingKind::Cycle;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiControlSpec {
    pub param_id: &'static str,
    pub section: &'static str,
}

impl UiControlSpec {
    pub const fn param(param_id: &'static str) -> Self {
        Self {
            param_id,
            section: "Main",
        }
    }

    pub const fn section(mut self, section: &'static str) -> Self {
        self.section = section;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AppSpec {
    pub name: &'static str,
    pub params: &'static [ParamSpec],
    pub hardware: &'static [HardwareBinding],
    pub ui: &'static [UiControlSpec],
}

impl AppSpec {
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            params: &[],
            hardware: &[],
            ui: &[],
        }
    }

    pub const fn params(mut self, params: &'static [ParamSpec]) -> Self {
        self.params = params;
        self
    }

    pub const fn hardware(mut self, hardware: &'static [HardwareBinding]) -> Self {
        self.hardware = hardware;
        self
    }

    pub const fn ui(mut self, ui: &'static [UiControlSpec]) -> Self {
        self.ui = ui;
        self
    }

    pub fn param(&self, id: &str) -> Option<&ParamSpec> {
        self.params.iter().find(|param| param.id == id)
    }
}

pub struct ParamBinding<T> {
    pub id: &'static str,
    pub get: fn(&T) -> ParamValue,
    pub set: fn(&mut T, ParamValue) -> Result<(), ParamError>,
}

pub fn get_bound_param<T>(
    bindings: &[ParamBinding<T>],
    target: &T,
    id: &str,
) -> Option<ParamValue> {
    bindings
        .iter()
        .find(|binding| binding.id == id)
        .map(|binding| (binding.get)(target))
}

pub fn set_bound_param<T>(
    bindings: &[ParamBinding<T>],
    target: &mut T,
    id: &str,
    value: ParamValue,
) -> Result<(), ParamError> {
    match bindings.iter().find(|binding| binding.id == id) {
        Some(binding) => (binding.set)(target, value),
        None => Err(ParamError::UnknownParam),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamError {
    UnknownParam,
    TypeMismatch,
}

pub trait ParamModel {
    fn app_spec(&self) -> &'static AppSpec;
    fn get_param(&self, id: &str) -> Option<ParamValue>;
    fn set_param(&mut self, id: &str, value: ParamValue) -> Result<(), ParamError>;

    fn set_normalized(&mut self, id: &str, normalized: f32) -> Result<(), ParamError> {
        let spec = self.app_spec().param(id).ok_or(ParamError::UnknownParam)?;
        match spec.kind {
            ParamKind::Knob { min, max, .. } => {
                let normalized = normalized.clamp(0.0, 1.0);
                let value = min + normalized * (max - min);
                self.set_param(id, ParamValue::Float(value))
            }
            ParamKind::Toggle { .. } => self.set_param(id, ParamValue::Bool(normalized >= 0.5)),
            ParamKind::Choice { options, .. } => {
                let len = options.len();
                let index = if len <= 1 {
                    0
                } else {
                    let scaled = normalized.clamp(0.0, 1.0) * len as f32;
                    let index = scaled as usize;
                    index.min(len - 1) as u8
                };
                self.set_param(id, ParamValue::Choice(index))
            }
        }
    }
}

impl<T: ParamModel + ?Sized> ParamModel for &mut T {
    fn app_spec(&self) -> &'static AppSpec {
        (**self).app_spec()
    }

    fn get_param(&self, id: &str) -> Option<ParamValue> {
        (**self).get_param(id)
    }

    fn set_param(&mut self, id: &str, value: ParamValue) -> Result<(), ParamError> {
        (**self).set_param(id, value)
    }
}

pub fn apply_hardware_binding<M: ParamModel>(
    model: &mut M,
    binding: &HardwareBinding,
    input: ParamValue,
) -> Result<(), ParamError> {
    match binding.binding_kind {
        HardwareBindingKind::SetNormalized => match input {
            ParamValue::Float(value) => model.set_normalized(binding.param_id, value),
            _ => Err(ParamError::TypeMismatch),
        },
        HardwareBindingKind::SetBool => match input {
            ParamValue::Bool(value) => model.set_param(binding.param_id, ParamValue::Bool(value)),
            _ => Err(ParamError::TypeMismatch),
        },
        HardwareBindingKind::Toggle => match model.get_param(binding.param_id) {
            Some(ParamValue::Bool(value)) => {
                if let ParamValue::Bool(pressed) = input {
                    if pressed {
                        model.set_param(binding.param_id, ParamValue::Bool(!value))
                    } else {
                        Ok(())
                    }
                } else {
                    Err(ParamError::TypeMismatch)
                }
            }
            _ => Err(ParamError::TypeMismatch),
        },
        HardwareBindingKind::Cycle => {
            if let ParamValue::Bool(pressed) = input {
                if !pressed {
                    return Ok(());
                }
            } else {
                return Err(ParamError::TypeMismatch);
            }

            match (
                model.app_spec().param(binding.param_id),
                model.get_param(binding.param_id),
            ) {
                (Some(spec), Some(ParamValue::Choice(current))) => {
                    if let ParamKind::Choice { options, .. } = spec.kind {
                        let next = if options.is_empty() {
                            0
                        } else {
                            (current as usize + 1) % options.len()
                        } as u8;
                        model.set_param(binding.param_id, ParamValue::Choice(next))
                    } else {
                        Err(ParamError::TypeMismatch)
                    }
                }
                _ => Err(ParamError::TypeMismatch),
            }
        }
    }
}

#[macro_export]
macro_rules! define_params {
    (
        $(
            $name:ident : toggle($id:literal, $label:literal, $group:literal, $default:expr)
        ),* $(,)?
    ) => {
        pub mod param {
            $(pub const $name: &str = $id;)*
        }

        pub const PARAMS: &[$crate::ParamSpec] = &[
            $(
                $crate::ParamSpec::toggle(param::$name)
                    .label($label)
                    .group($group)
                    .default_bool($default)
            ),*
        ];
    };
    (
        $(
            $name:ident : $kind:ident($($args:tt)*)
        ),* $(,)?
    ) => {
        pub mod param {
            $(pub const $name: &str = $crate::define_params!(@id $kind($($args)*));)*
        }

        pub const PARAMS: &[$crate::ParamSpec] = &[
            $($crate::define_params!(@spec $name : $kind($($args)*))),*
        ];
    };
    (@id toggle($id:literal, $label:literal, $group:literal, $default:expr)) => { $id };
    (@id knob($id:literal, $label:literal, $group:literal, $min:expr, $max:expr, $default:expr)) => { $id };
    (@id choice($id:literal, $label:literal, $group:literal, $options:expr, $default:expr)) => { $id };
    (@spec $name:ident : toggle($id:literal, $label:literal, $group:literal, $default:expr)) => {
        $crate::ParamSpec::toggle(param::$name)
            .label($label)
            .group($group)
            .default_bool($default)
    };
    (@spec $name:ident : knob($id:literal, $label:literal, $group:literal, $min:expr, $max:expr, $default:expr)) => {
        $crate::ParamSpec::knob(param::$name)
            .label($label)
            .group($group)
            .range($min, $max)
            .default_float($default)
    };
    (@spec $name:ident : choice($id:literal, $label:literal, $group:literal, $options:expr, $default:expr)) => {
        $crate::ParamSpec::choice(param::$name, $options)
            .label($label)
            .group($group)
            .default_choice($default)
    };
}

#[macro_export]
macro_rules! define_ui_controls {
    ($(($param:expr, $section:literal)),* $(,)?) => {
        pub const UI_CONTROLS: &[$crate::UiControlSpec] = &[
            $($crate::UiControlSpec::param($param).section($section)),*
        ];
    };
}

#[macro_export]
macro_rules! define_hardware_bindings {
    ($(pot($control:literal, $param:expr, $page:literal)),* $(,)?) => {
        pub const DAISY_BINDINGS: &[$crate::HardwareBinding] = &[
            $(
                $crate::HardwareBinding::pot($control)
                    .bind($param)
                    .normalized()
                    .page($page)
            ),*
        ];
    };
    (
        $(
            $kind:ident($control:literal, $param:expr, $binding:ident, $page:literal)
        ),* $(,)?
    ) => {
        pub const DAISY_BINDINGS: &[$crate::HardwareBinding] = &[
            $(
                $crate::HardwareBinding::$kind($control)
                    .bind($param)
                    .$binding()
                    .page($page)
            ),*
        ];
    };
}
