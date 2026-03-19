import { UserRepository } from "../../src/repositories/UserRepository";
import { validateToken } from "../../src/services/AuthService";
import { UserService } from "../../src/services/UserService";

describe("userFlow", () => {
  it("full user creation flow", () => {
    const repo = new UserRepository();
    const service = new UserService(repo);

    const created = service.createUser({
      id: "user-flow-1",
      email: "flow@example.com",
      name: "Flow User",
      passwordHash: "pw-flow"
    });
    expect(created.email).toBe("flow@example.com");

    const fetched = service.getUser("user-flow-1");
    expect(fetched?.name).toBe("Flow User");

    const isTokenValid = validateToken("token-1234567890");
    expect(isTokenValid).toBe(true);

    const deleted = service.deleteUser("user-flow-1");
    expect(deleted).toBe(true);
  });
});
