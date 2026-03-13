import { User } from "../src/models/User";
import { UserRepository } from "../src/repositories/UserRepository";

describe("UserRepository", () => {
  const baseUser: User = {
    id: "user-1",
    email: "user@example.com",
    name: "User One",
    passwordHash: "pw-1"
  };

  it("should find user by id", () => {
    const repo = new UserRepository([baseUser]);

    const found = repo.findById("user-1");

    expect(found).toEqual(baseUser);
    expect(repo.findById("missing")).toBeNull();
  });

  it("should find user by email", () => {
    const repo = new UserRepository([baseUser]);

    const found = repo.findByEmail("user@example.com");

    expect(found?.id).toBe("user-1");
    expect(repo.findByEmail("missing@example.com")).toBeNull();
  });

  it("should delete existing user", () => {
    const repo = new UserRepository([baseUser]);

    const deleted = repo.delete("user-1");

    expect(deleted).toBe(true);
    expect(repo.findById("user-1")).toBeNull();
  });
});
