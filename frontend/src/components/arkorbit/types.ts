export type OrbitId = string;

export type OrbitMetadata = {
  icon?: string;
  color?: string;
  agent_instructions?: string;
};

export type Orbit = OrbitMetadata & {
  id: OrbitId;
  name: string;
  is_default?: boolean;
  created_at?: string;
  updated_at?: string;
};

export type OrbitsResponse = {
  orbits: Orbit[];
};

export type OrbitPatch = {
  name?: string;
  icon?: string | null;
  color?: string | null;
  agent_instructions?: string | null;
};

export type OrbitChatFileChip = {
  id: string;
  path: string;
  operation?: "wrote" | "edited";
  bytes?: number;
};

export type OrbitFileEntry = {
  path: string;
  bytes: number;
};

export type OrbitChatUsage = {
  model?: string;
  input_tokens?: number;
  output_tokens?: number;
  total_tokens?: number;
  cached_prompt_tokens?: number;
  cache_creation_prompt_tokens?: number;
  cost_usd?: number;
  estimated?: boolean;
  duration_ms?: number;
  time_to_first_token_ms?: number;
};

export type OrbitChatMessageStatus = "running" | "completed" | "failed" | "stopped";

export type OrbitChatHistoryMessage = OrbitChatUsage & {
  id: string;
  role: "user" | "assistant" | string;
  content: string;
  created_at?: string;
  status?: OrbitChatMessageStatus;
  activity?: string;
};

export type OrbitChatTranscript = {
  id: string;
  title: string;
  created_at?: string;
  updated_at?: string;
  message_count: number;
  current?: boolean;
};
