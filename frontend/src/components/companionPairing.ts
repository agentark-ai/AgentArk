type PairingSessionLike = {
  id: string;
  status: string;
};

type CompanionDevicesLike = {
  pairing_sessions?: PairingSessionLike[];
};

const ACTIVE_PAIRING_STATUSES = new Set(["pending", "claimed", "approved"]);

export function sessionNeedsPairingPoll(data: CompanionDevicesLike | undefined, sessionId: string): boolean {
  if (!sessionId) return false;
  const session = data?.pairing_sessions?.find((item) => item.id === sessionId);
  if (!session) return true;
  return ACTIVE_PAIRING_STATUSES.has(session.status.toLowerCase());
}
