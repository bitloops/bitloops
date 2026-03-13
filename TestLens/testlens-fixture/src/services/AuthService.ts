export function validateToken(token: string): boolean {
  return token.startsWith("token-") && token.length > 10;
}

// Intentionally left untested to validate the `untested` query path.
export function hashPassword(raw: string): string {
  let hash = 0;
  for (let index = 0; index < raw.length; index += 1) {
    hash = (hash << 5) - hash + raw.charCodeAt(index);
    hash |= 0;
  }
  return `h${Math.abs(hash).toString(16)}`;
}
