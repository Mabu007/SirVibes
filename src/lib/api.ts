import { invoke } from "@tauri-apps/api/core";
import type {
  ApiInput,
  ApiUsage,
  ApiView,
  Artifact,
  AssistantMessage,
  Capability,
  ChatMessage,
  Conversation,
  ConversationSummary,
  Evaluation,
  ImportReport,
  ModelInfo,
  SettingsPatch,
  SettingsView,
  Skill,
  SkillDir,
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
  listCapabilities: () => invoke<Capability[]>("list_capabilities"),
  getSystemPrompt: () => invoke<string>("get_system_prompt"),

  evaluateTool: (tool: string, args: unknown) => invoke<Evaluation>("evaluate_tool", { tool, args }),
  runTool: (tool: string, args: unknown, callId: string, userApproved: boolean) =>
    invoke<{ ok: boolean; result?: unknown; error?: string }>("run_tool", {
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

  scanArtifacts: (sinceMs: number) => invoke<Artifact[]>("scan_artifacts", { sinceMs }),
  openPath: (path: string) => invoke<void>("open_path", { path }),
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),

  saveConversation: (id: string, data: Conversation) =>
    invoke<void>("save_conversation", { id, data }),
  listConversations: () => invoke<ConversationSummary[]>("list_conversations"),
  loadConversation: (id: string) => invoke<Conversation>("load_conversation", { id }),
  deleteConversation: (id: string) => invoke<void>("delete_conversation", { id }),
};
