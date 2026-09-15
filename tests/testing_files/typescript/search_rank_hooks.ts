/**
 * Fixture for the TypeScript search-ranking E2E suite. Deliberately models
 * the reported failure mode: `useChangePassword` has no doc comment, so its
 * behaviour is only observable through the API call in its body, while the
 * surrounding constants carry prose rich in "change / password / user"
 * vocabulary.
 *
 * For the paraphrase "change password current user me hook" the ranking
 * contract (v1.9.7) promotes `submitChangePassword` — the orchestrator that
 * calls the top-ranked helpers `run` / `clearSession` /
 * `postSelfChangePassword` — to #1 via root-set coverage, and requires the
 * hook itself to stay within position 2 (the "entry point ≤ 2" rule).
 */

/**
 * Fetches the JSON schema the user must satisfy when choosing a new
 * password in the profile form.
 */
export const getSelfChangePasswordSchema = () => null;

/** Clears the session cookie and local cache. */
export function clearSession(): void {}

/**
 * Submits the user's new password after validating it against the schema.
 * Reads the current password to confirm it belongs to the same user.
 */
export function submitChangePassword(
  currentPassword: string,
  newPassword: string,
): Promise<void> {
  void currentPassword;
  async function run(): Promise<void> {
    await postSelfChangePassword(currentPassword, newPassword);
    clearSession();
  }
  return run();
}

async function postSelfChangePassword(
  currentPassword: string,
  newPassword: string,
): Promise<void> {
  void currentPassword;
  void newPassword;
}

// No doc comment on purpose: this is the fixture's definition-under-test.
export function useChangePassword() {
  return { submit: submitChangePassword };
}
