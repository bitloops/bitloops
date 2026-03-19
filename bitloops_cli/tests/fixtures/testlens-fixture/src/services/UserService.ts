import { MAX_NAME_LENGTH, User, UserId } from "../models/User";
import { UserRepository } from "../repositories/UserRepository";

export interface CreateUserInput {
  id: UserId;
  email: string;
  name: string;
  passwordHash: string;
}

export class UserService {
  constructor(private readonly repo: UserRepository) {}

  createUser(input: CreateUserInput): User {
    if (!input.email.includes("@")) {
      throw new Error("Email must be valid");
    }
    if (input.name.length > MAX_NAME_LENGTH) {
      throw new Error("Name is too long");
    }
    if (this.repo.findByEmail(input.email)) {
      throw new Error("Email already exists");
    }

    const user: User = {
      id: input.id,
      email: input.email,
      name: input.name,
      passwordHash: input.passwordHash
    };
    this.repo.save(user);
    return user;
  }

  getUser(id: UserId): User | null {
    return this.repo.findById(id);
  }

  deleteUser(id: UserId): boolean {
    return this.repo.delete(id);
  }
}
