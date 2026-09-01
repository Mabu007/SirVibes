export type PermissionMode = "ask" | "smart" | "full";

export interface SettingsView {
  api_key_set: boolean;
  api_key_hint: string;
  deepgram_key_set: boolean;
  deepgram_key_hint: string;
  model: string;
  /** The model every `see` call runs on, default included. */
  vision_model: string;
  /** The model that watches reference videos where they live. */
  reference_model: string;
  permission_mode: PermissionMode;
  workspace: string | null;
  skill_dirs: string[];
  recent_workspaces: string[];
  shell_timeout_secs: number;
}

export interface SettingsPatch {
  api_key?: string;
  deepgram_api_key?: string;
  model?: string;
  vision_model?: string;
  reference_model?: string;
  permission_mode?: PermissionMode;
  workspace?: string;
  skill_dirs?: string[];
  shell_timeout_secs?: number;
}

export interface ModelInfo {
  id: string;
  name: string;
  context_length: number;
  prompt_price: string;
  supports_tools: boolean;
  /** The organisation the model comes from — the id's prefix. */
  provider: string;
  input_modalities: string[];
  /** What it produces. This is what makes it an image or audio model. */
  output_modalities: string[];
  completion_price: string;
  description: string;
}

export interface Skill {
  name: string;
  description: string;
  when_to_use: string;
  path: string;
  source: string;
}

export interface SkillDir {
  path: string;
  source: string;
  exists: boolean;
}

/** One skill that landed in the skills folder, and what it did on the way. */
export interface Imported {
  name: string;
  path: string;
  replaced: boolean;
}

export interface ImportFailure {
  source: string;
  reason: string;
}

export interface ImportReport {
  imported: Imported[];
  failed: ImportFailure[];
}

export interface Capability {
  name: string;
  available: boolean;
  detail: string;
}

export interface Risk {
  kind: string;
  message: string;
}

export interface Evaluation {
  decision: "allow" | "ask" | "deny";
  title: string;
  detail: string;
  risks: Risk[];
}

export interface Artifact {
  name: string;
  path: string;
  absolute_path: string;
  size: number;
  modified_ms: number;
  kind: "video" | "audio" | "image" | "subtitles" | "document";
}

export interface ToolCall {
  id: string;
  name: string;
  arguments: string;
}

export interface AssistantMessage {
  content: string;
  reasoning: string;
  tool_calls: ToolCall[];
  finish_reason: string | null;
  usage: unknown;
  model: string | null;
}

/** Messages exactly as the model sees them. */
export type ChatMessage =
  | { role: "system" | "user"; content: string }
  | {
      role: "assistant";
      content: string;
      tool_calls?: { id: string; type: "function"; function: { name: string; arguments: string } }[];
    }
  | { role: "tool"; tool_call_id: string; content: string };

/** One thing the user can choose, described as an outcome rather than a method. */
export interface QuestionOption {
  label: string;
  detail?: string;
}

/** A question the agent stopped to ask, and the answer once it has one. */
export interface Question {
  question: string;
  context?: string;
  options: QuestionOption[];
  allowOther: boolean;
  answer?: string;
}

/** A reading off a command's own progress output, already made readable. */
export interface ToolProgress {
  /** "Streaming frame — 55% · 207/375" */
  summary: string;
  label: string;
  percent?: number;
  done?: number;
  total?: number;
  /** How many redraws this one reading stands for. */
  updates: number;
}

export type ToolStatus =
  | "awaiting"
  | "running"
  | "ok"
  | "error"
  | "denied"
  | "cancelled";

/** What the conversation view renders. */
export type Item =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string; reasoning: string; streaming: boolean }
  | {
      kind: "tool";
      id: string;
      callId: string;
      name: string;
      title: string;
      detail: string;
      purpose: string;
      status: ToolStatus;
      /** Where a long-running command has got to, if it reports progress. */
      progress?: ToolProgress;
      /** Set when this call is the agent asking the user something. */
      question?: Question;
      evaluation?: Evaluation;
      summary: string;
      output: string[];
      resultText: string;
      durationMs?: number;
    }
  | { kind: "artifacts"; id: string; items: Artifact[] }
  | { kind: "error"; id: string; text: string };

export interface Conversation {
  id: string;
  title: string;
  updated_ms: number;
  workspace: string | null;
  items: Item[];
  messages: ChatMessage[];
}

export interface ConversationSummary {
  id: string;
  title: string;
  updated_ms: number;
  workspace: string | null;
}

export interface ApiTestResult {
  ok: boolean;
  message: string;
  tested_ms: number;
}

/** What the interface is allowed to know about a connection. Never the key. */
export interface ApiView {
  id: string;
  name: string;
  base_url: string | null;
  doc_url: string | null;
  auth_kind: string;
  notes: string;
  created_ms: number;
  updated_ms: number;
  key_hint: string;
  has_credential: boolean;
  capability_count: number;
  doc_source: string;
  has_docs: boolean;
  /** The docs link is saved but unread; it is read on first use. */
  docs_pending: boolean;
  /** No base URL, so the agent has nowhere to send a request. */
  needs_base_url: boolean;
  status: "connected" | "failed" | "untested" | "no credential";
  last_test: ApiTestResult | null;
}

export interface ApiInput {
  id?: string;
  name: string;
  api_key?: string;
  doc_url?: string;
  base_url?: string;
  notes?: string;
  auth?:
    | { kind: "bearer" }
    | { kind: "header"; name: string; prefix: string }
    | { kind: "query_param"; name: string }
    | { kind: "none" };
}

export interface ApiUsage {
  api_id: string;
  calls: number;
  errors: number;
  bytes_received: number;
  last_ms: number;
}

// ------------------------------------------------------------ connected apps

/** One application Composio can broker a connection to. */
export interface Toolkit {
  slug: string;
  name: string;
  logo: string | null;
  categories: string[];
  tools_count: number;
  no_auth: boolean;
  composio_managed_auth_schemes: string[];
  /** False when the app needs OAuth credentials registered in Composio first. */
  connectable: boolean;
}

/**
 * One app this user has connected. Carries no connection handle and no user
 * id — those stay in the backend.
 */
export interface AppView {
  toolkit_slug: string;
  name: string;
  logo: string | null;
  status: string;
  status_reason: string | null;
  connected: boolean;
  /** Sign-in was started but has not finished. */
  pending: boolean;
  connected_ms: number;
  updated_ms: number;
}

/** Whether connected apps are set up at all. Never carries the key. */
export interface AppsStatus {
  configured: boolean;
  key_hint: string;
  /** The key came from COMPOSIO_API_KEY, so there is nothing to edit here. */
  from_environment: boolean;
}

export interface ConnectStarted {
  toolkit_slug: string;
  name: string;
  redirect_url: string;
  expires_at: string | null;
}
