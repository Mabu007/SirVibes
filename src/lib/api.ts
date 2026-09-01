import { invoke } from "@tauri-apps/api/core";
import type {
  ApiInput,
  ApiUsage,
  ApiView,
  AppsStatus,
  SystemUsage,
  AppView,
  Artifact,
  AssistantMessage,
  Capability,
  ChatMessage,
  Conversation,
  ConversationSummary,
  Evaluation,
  ImportReport,
  ConnectStarted,
  ModelInfo,
  SettingsPatch,
  SettingsView,
  Skill,
  SkillDir,
  Toolkit,
} from "./types";

export const api = {
  getSettings: () => invoke<SettingsView>("get_settings"),
  updateSettings: (patch: SettingsPatch) => invoke<SettingsView>("update_settings", { patch }),
  listModels: () => invoke<ModelInfo[]>("list_models"),

  listSkills: () => invoke<Skill[]>("list_skills"),
  getSkillDirs: () => invoke<SkillDir[]>("get_skill_dirs"),
  ensureUserSkillsDir: () => invoke<string>("ensure_user_skills_dir"),
  skillRead: (path: string) => invoke<string>("skill_read", { path }),
  skillWrite: (name: string, content: string) =>
    invoke<string>("skill_write", { name, content }),
  skillDelete: (path: string) => invoke<void>("skill_delete", { path }),
  skillImport: (sources: string[]) => invoke<ImportReport>("skill_import", { sources }),
  generateText: (prompt: string, system: string) =>
    invoke<string>("generate_text", { prompt, system }),
  systemUsage: () => invoke<SystemUsage>("system_usage"),

  listCapabilities: () => invoke<Capability[]>("list_capabilities"),
  getSystemPrompt: () => invoke<string>("get_system_prompt"),

  evaluateTool: (tool: string, args: unknown) => invoke<Evaluation>("evaluate_tool", { tool, args }),
  runTool: (tool: string, args: unknown, callId: string, userApproved: boolean) =>
    invoke<{ ok: boolean; cancelled?: boolean; result?: unknown; error?: string }>("run_tool", {
      tool,
      args,
      callId,
      userApproved,
    }),

  chatStream: (messages: ChatMessage[], streamId: string) =>
    invoke<AssistantMessage>("chat_stream", { messages, streamId }),
  cancelStream: (streamId: string) => invoke<void>("cancel_stream", { streamId }),
  cancelTool: (callId: string) => invoke<boolean>("cancel_tool", { callId }),

  apiList: () => invoke<ApiView[]>("api_list"),
  apiGet: (id: string) => invoke<ApiView>("api_get", { id }),
  apiAdd: (input: ApiInput) => invoke<ApiView>("api_add", { input }),
  apiUpdate: (input: ApiInput) => invoke<ApiView>("api_update", { input }),
  apiDelete: (id: string) => invoke<void>("api_delete", { id }),
  apiRediscover: (id: string) => invoke<ApiView>("api_rediscover", { id }),
  apiTest: (id: string) => invoke<ApiView>("api_test", { id }),
  apiUsage: () => invoke<ApiUsage[]>("api_usage"),

  // Connected apps. The Composio key stays in the backend; only whether one is
  // configured ever crosses this boundary.
  appsStatus: () => invoke<AppsStatus>("apps_status"),
  appsSetKey: (key: string) => invoke<AppsStatus>("apps_set_key", { key }),
  appsClearKey: () => invoke<AppsStatus>("apps_clear_key"),
  appsCatalog: (search: string) => invoke<Toolkit[]>("apps_catalog", { search }),
  appsList: () => invoke<AppView[]>("apps_list"),
  appsRefresh: () => invoke<AppView[]>("apps_refresh"),
  appsConnect: (toolkitSlug: string) =>
    invoke<ConnectStarted>("apps_connect", { toolkitSlug }),
  appsCheck: (toolkitSlug: string) => invoke<AppView>("apps_check", { toolkitSlug }),
  appsDisconnect: (toolkitSlug: string) => invoke<void>("apps_disconnect", { toolkitSlug }),

  scanArtifacts: (sinceMs: number) => invoke<Artifact[]>("scan_artifacts", { sinceMs }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),

  saveConversation: (id: string, data: Conversation) =>
    invoke<void>("save_conversation", { id, data }),
  listConversations: () => invoke<ConversationSummary[]>("list_conversations"),
  loadConversation: (id: string) => invoke<Conversation>("load_conversation", { id }),
  deleteConversation: (id: string) => invoke<void>("delete_conversation", { id }),
};
