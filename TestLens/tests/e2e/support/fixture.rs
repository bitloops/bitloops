use std::fs;
use std::path::{Path, PathBuf};

use tempfile::TempDir;

#[derive(Debug)]
pub struct BddWorkspace {
    _temp_dir: TempDir,
    repo_dir: PathBuf,
    db_path: PathBuf,
}

impl BddWorkspace {
    pub fn new() -> Self {
        let temp_dir = TempDir::new().expect("failed to create temp dir for BDD workspace");
        let repo_dir = temp_dir.path().join("fixture-repo");
        let db_path = temp_dir.path().join("testlens.db");

        Self {
            _temp_dir: temp_dir,
            repo_dir,
            db_path,
        }
    }

    pub fn repo_dir(&self) -> &Path {
        &self.repo_dir
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }

    pub fn write_file(&self, relative_path: &str, content: &str) {
        let target = self.repo_dir.join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).expect("failed to create parent directory");
        }
        fs::write(target, content.trim_start()).expect("failed to write fixture file");
    }
}

pub fn write_cli_1345_base_fixture(workspace: &BddWorkspace) {
    workspace.write_file(
        "src/service.ts",
        r#"
export function doThing(input: string): string {
  return input.trim().toUpperCase();
}
"#,
    );
    workspace.write_file(
        "src/lib.rs",
        r#"
pub fn normalize(input: &str) -> String {
    input.trim().to_lowercase()
}
"#,
    );
    workspace.write_file(
        "tests/base.test.ts",
        r#"
import { doThing } from "../src/service";

describe("service", () => {
  it("normalizes input", () => {
    expect(doThing(" hello ")).toBe("HELLO");
  });
});
"#,
    );
    workspace.write_file(
        "tests/rust_unit.rs",
        r#"
#[cfg(test)]
mod tests {
    #[test]
    fn sample() {
        assert_eq!(2 + 2, 4);
    }
}
"#,
    );
}

pub fn write_cli_1345_c1_extra_test(workspace: &BddWorkspace) {
    workspace.write_file(
        "tests/new.test.ts",
        r#"
import { doThing } from "../src/service";

describe("service c1", () => {
  it("adds another scenario", () => {
    expect(doThing("world")).toBe("WORLD");
  });
});
"#,
    );
}

pub fn write_cli_1346_base_fixture(workspace: &BddWorkspace) {
    workspace.write_file(
        "src/repositories/UserRepository.ts",
        r#"
export class UserRepository {
  findById(id: number): string | null {
    return id > 0 ? `user-${id}` : null;
  }

  findByEmail(email: string): string | null {
    return email.includes("@") ? email : null;
  }
}
"#,
    );

    workspace.write_file(
        "src/repositories/user_repository.rs",
        r#"
#[derive(Debug, Default)]
pub struct UserRepository;

impl UserRepository {
    pub fn new() -> Self {
        Self
    }

    pub fn find_by_id(&self, id: u32) -> Option<String> {
        (id > 0).then(|| format!("user-{}", id))
    }

    pub fn find_by_email(&self, email: &str) -> Option<String> {
        email.contains('@').then(|| email.to_string())
    }
}
"#,
    );

    workspace.write_file(
        "tests/userRepository.test.ts",
        r#"
import { UserRepository } from "../src/repositories/UserRepository";

describe("ts repo", () => {
  it("finds by id", () => {
    const repo = new UserRepository();
    repo.findById(1);
  });

  it("calls email lookup only", () => {
    const repo = new UserRepository();
    repo.findByEmail("foo@bar.com");
  });
});
"#,
    );

    workspace.write_file(
        "tests/rust_repo_test.rs",
        r#"
use crate::repositories::user_repository::UserRepository;

#[cfg(test)]
mod tests {
    use super::UserRepository;

    #[test]
    fn finds_by_id() {
        let repo = UserRepository::new();
        let _ = repo.find_by_id(1);
    }

    #[test]
    fn calls_email_lookup_only() {
        let repo = UserRepository::new();
        let _ = repo.find_by_email("foo@bar.com");
    }
}
"#,
    );
}

pub fn write_cli_1346_c1_updated_tests(workspace: &BddWorkspace) {
    workspace.write_file(
        "tests/userRepository.test.ts",
        r#"
import { UserRepository } from "../src/repositories/UserRepository";

describe("ts repo", () => {
  it("finds by id", () => {
    const repo = new UserRepository();
    repo.findById(1);
    repo.findByEmail("c1@bar.com");
  });

  it("calls email lookup only", () => {
    const repo = new UserRepository();
    repo.findByEmail("foo@bar.com");
  });
});
"#,
    );

    workspace.write_file(
        "tests/rust_repo_test.rs",
        r#"
use crate::repositories::user_repository::UserRepository;

#[cfg(test)]
mod tests {
    use super::UserRepository;

    #[test]
    fn finds_by_id() {
        let repo = UserRepository::new();
        let _ = repo.find_by_id(1);
        let _ = repo.find_by_email("c1@bar.com");
    }

    #[test]
    fn calls_email_lookup_only() {
        let repo = UserRepository::new();
        let _ = repo.find_by_email("foo@bar.com");
    }
}
"#,
    );
}
