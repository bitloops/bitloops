fn main() {
    cc::Build::new()
        .file("sqlite-vec.c")
        .define("SQLITE_CORE", None)
        // sqlite-vec 0.1.10-alpha.3 omits these optional implementation files
        // from the published crate package.
        .define("SQLITE_VEC_ENABLE_DISKANN", Some("0"))
        .define("SQLITE_VEC_ENABLE_RESCORE", Some("0"))
        .warnings(false)
        .flag_if_supported("-w")
        .flag_if_supported("/w")
        .compile("sqlite_vec0");
}
