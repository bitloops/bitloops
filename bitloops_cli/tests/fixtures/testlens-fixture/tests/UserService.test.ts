import { UserRepository } from "../src/repositories/UserRepository";
import { UserService } from "../src/services/UserService";

describe("UserService", () => {
  it("should create user", () => {
    const repo = new UserRepository();
    const service = new UserService(repo);

    const user = service.createUser({
      id: "user-2",
      email: "new@example.com",
      name: "New User",
      passwordHash: "pw-2"
    });

    expect(user.id).toBe("user-2");
    expect(repo.findByEmail("new@example.com")?.id).toBe("user-2");
  });

  // Intentional failure to validate the pre-existing failure path in the PRD.
  it("should reject duplicate email", () => {
    const repo = new UserRepository();
    const service = new UserService(repo);

    service.createUser({
      id: "user-3",
      email: "dup@example.com",
      name: "Dupe One",
      passwordHash: "pw-3"
    });

    expect(() =>
      service.createUser({
        id: "user-4",
        email: "dup@example.com",
        name: "Dupe Two",
        passwordHash: "pw-4"
      })
    ).toThrow("duplicate email");
  });
});
