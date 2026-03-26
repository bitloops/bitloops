pub(crate) mod rust;

use crate::host::language_adapter::LanguageAdapterPack;

pub(crate) fn builtin_language_adapter_packs() -> Vec<Box<dyn LanguageAdapterPack>> {
    vec![
        Box::new(rust::pack::RustLanguageAdapterPack),
    ]
}
