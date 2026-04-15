pub(crate) mod csharp;
pub(crate) mod go;
pub(crate) mod java;
pub(crate) mod python;
pub(crate) mod rust;
pub(crate) mod ts_js;

use crate::host::language_adapter::LanguageAdapterPack;

pub(crate) fn builtin_language_adapter_packs() -> Vec<Box<dyn LanguageAdapterPack>> {
    vec![
        Box::new(rust::pack::RustLanguageAdapterPack),
        Box::new(ts_js::pack::TsJsLanguageAdapterPack),
        Box::new(python::pack::PythonLanguageAdapterPack),
        Box::new(go::pack::GoLanguageAdapterPack),
        Box::new(java::pack::JavaLanguageAdapterPack),
        Box::new(csharp::pack::CSharpLanguageAdapterPack),
    ]
}

#[cfg(test)]
mod tests {
    use super::builtin_language_adapter_packs;

    #[test]
    fn builtin_language_adapter_packs_include_go_and_python() {
        let pack_ids = builtin_language_adapter_packs()
            .into_iter()
            .map(|pack| pack.descriptor().id)
            .collect::<Vec<_>>();

        assert_eq!(
            pack_ids,
            vec![
                "rust-language-pack",
                "ts-js-language-pack",
                "python-language-pack",
                "go-language-pack",
                "java-language-pack",
                "csharp-language-pack",
            ]
        );
    }
}
