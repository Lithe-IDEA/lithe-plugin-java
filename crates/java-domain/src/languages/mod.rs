//! Source-level Java, Spring and MyBatis analysis that does not need a
//! language server: structure, code vision, class names, Spring indexes and
//! MyBatis mapper pairing.

mod code_vision_hints;
mod java;
mod java_syntax;
mod mybatis;
mod spring;
mod text;

pub use code_vision_hints::*;
pub use java::*;
pub use mybatis::*;
pub use spring::*;
pub use text::*;
