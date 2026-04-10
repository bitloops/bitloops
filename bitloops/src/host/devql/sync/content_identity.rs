use sha1::{Digest, Sha1};

pub(crate) fn compute_blob_oid(content: &[u8]) -> String {
    let header = format!("blob {}\0", content.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(content);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::compute_blob_oid;

    #[test]
    fn blob_oid_matches_git_for_known_content() {
        assert_eq!(
            compute_blob_oid(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
    }

    #[test]
    fn blob_oid_empty_file() {
        assert_eq!(
            compute_blob_oid(b""),
            "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
        );
    }

    #[test]
    fn identical_content_produces_same_oid() {
        let first = compute_blob_oid(b"same content\n");
        let second = compute_blob_oid(b"same content\n");

        assert_eq!(first, second);
    }
}
