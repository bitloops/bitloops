import { User, UserId } from "../models/User";

export class UserRepository {
  private readonly usersById = new Map<UserId, User>();
  private readonly usersByEmail = new Map<string, UserId>();

  constructor(seedUsers: User[] = []) {
    for (const user of seedUsers) {
      this.save(user);
    }
  }

  save(user: User): void {
    this.usersById.set(user.id, user);
    this.usersByEmail.set(user.email, user.id);
  }

  findById(id: UserId): User | null {
    if (!id) {
      return null;
    }

    const user = this.usersById.get(id);
    if (!user) {
      return null;
    }

    return user;
  }

  findByEmail(email: string): User | null {
    if (!email) {
      return null;
    }

    const userId = this.usersByEmail.get(email);
    if (!userId) {
      return null;
    }

    return this.findById(userId);
  }

  delete(id: UserId): boolean {
    const existing = this.usersById.get(id);
    if (!existing) {
      return false;
    }

    this.usersById.delete(id);
    this.usersByEmail.delete(existing.email);
    return true;
  }
}
