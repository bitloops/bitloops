export type UserId = string;

export interface User {
  id: UserId;
  email: string;
  name: string;
  passwordHash: string;
}

export const MAX_NAME_LENGTH = 64;
