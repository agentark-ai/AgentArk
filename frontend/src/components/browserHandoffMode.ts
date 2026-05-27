export type BrowserHandoffModeInput = {
  conversation_id?: string | null;
  profile_id?: string | null;
};

function hasText(value?: string | null): boolean {
  return Boolean(String(value || "").trim());
}

export function isProfileBrowserSession(status?: BrowserHandoffModeInput | null): boolean {
  return Boolean(status && hasText(status.profile_id) && !hasText(status.conversation_id));
}
