import * as fs from 'node:fs';
import * as path from 'node:path';
import * as yaml from 'js-yaml';

const TASK_ID_REGEX = /^CLO-\d+$/i;

type ExtensionAPI = any;

interface WorkflowHistoryEvent {
  timestamp: string;
  action: string;
  phase: string;
  details: string;
}

interface LinearBlock {
  team?: string;
  project?: string;
  status_at_start?: string;
  priority?: string | number;
  branch_suggested?: string;
  branch_actual?: string;
  blocks?: string[];
  blocked_by?: string[];
}

interface WorkflowState {
  task_id?: string;
  task_title?: string;
  task_url?: string;
  task_type?: 'development' | 'specification' | 'operational';
  classification_reason?: string;
  task_profile?: {
    has_backend?: boolean;
    has_frontend?: boolean;
    has_data_model?: boolean;
    has_external_deps?: boolean;
    skip_probe?: boolean;
  };
  pending_human_action?: {
    type?: string;
    message?: string;
    since?: string;
    context?: Record<string, any>;
  } | null;
  linear?: LinearBlock;
  workflow?: {
    current_phase?: string;
    status?: 'active' | 'blocked' | 'paused' | 'complete' | 'in_progress' | 'checkpoint';
    created_at?: string;
    updated_at?: string;
  };
  phases?: {
    discovery?: {
      status?: string;
      approved?: boolean;
      problem_statement?: string;
      selected_approach?: string;
      prior_art_searched?: boolean;
      approach_reasoning?: string;
      skipped_probe?: boolean;
      reason?: string;
      skip_reason?: string;
    };
    spec?: {
      status?: string;
      spec_file?: string;
      approved?: boolean;
      auto_approved?: boolean;
      auto_approval_reason?: string;
      review_completed?: boolean;
      review_skip_reason?: string;
      review_gemini?: string | null;
      review_ollama?: string | null;
      review_synthesis?: string | null;
      review_verdict?: string | null;
      review_applied?: boolean;
      applied_suggestions?: string[];
      flagged_suggestions?: string[];
      skip_reason?: string;
    };
    design?: {
      status?: string;
      reason?: string;
      skip_reason?: string;
      design_doc?: string;
      draft_ready?: boolean;
      finalized?: boolean;
      review_completed?: boolean;
      probe_completed?: boolean;
      probe_decision?: string | null;
      review_gemini?: string | null;
      review_ollama?: string | null;
      review_verdict?: string | null;
      review_applied?: boolean;
      applied_suggestions?: string[];
      flagged_suggestions?: string[];
    };
    plan?: {
      status?: string;
      reason?: string;
      skip_reason?: string;
      plan_file?: string;
      approved?: boolean;
    };
    implement?: {
      status?: string;
      last_phase_completed?: string;
      commits?: string[];
      codex_validated?: boolean;
      codex_verdict?: string;
      codex_report?: string;
      gemini_validation_report?: string;
    };
    pr?: {
      status?: string;
      pr_url?: string;
      pr_number?: number | string;
      ci_passed?: boolean;
      reviews_addressed?: number;
      approved?: boolean;
      merged_at?: string | null;
      merge_commit?: string | null;
    };
    complete?: {
      status?: string;
      aggregation_files_updated?: boolean;
      aggregation_files_skip_reason?: string;
      merged_at?: string | null;
      completed_at?: string | null;
    };
    execute?: {
      status?: string;
      findings?: string;
      steps_completed?: string[];
    };
    document?: {
      status?: string;
      doc_file?: string;
      lessons_learned?: string[];
    };
  };
  history?: WorkflowHistoryEvent[];
}

// Strict phase transition graph - mirrors loker's Claude flow (no separate review phase).
const ALLOWED_TRANSITIONS: Record<string, string[]> = {
  init: ['discovery', 'spec', 'operational'],
  discovery: ['design'],
  spec: ['implement'],
  operational: ['execute', 'document', 'complete'],
  design: ['plan'],
  plan: ['implement'],
  implement: ['pr'],
  pr: ['complete'],
  execute: ['document', 'complete'],
  document: ['complete', 'pr'],
  complete: []
};

const TYPE_ALLOWED_PHASES: Record<string, Set<string>> = {
  development: new Set(['init', 'discovery', 'design', 'plan', 'implement', 'pr', 'complete']),
  specification: new Set(['init', 'spec', 'implement', 'pr', 'complete']),
  operational: new Set(['init', 'operational', 'execute', 'document', 'pr', 'complete']),
};

// Required fields and history events match docs/.claude/commands/task/orchestrate.md
// and the per-phase YAML Checkpoint sections in .claude/commands/task/phases/*.md.
const PHASE_CONFIG: Record<string, { requiredFields: string[]; historyEvents: string[] }> = {
  discovery: {
    requiredFields: ['status'],
    historyEvents: ['discovery_approved']
  },
  spec: {
    requiredFields: ['status', 'spec_file', 'approved', 'review_completed'],
    historyEvents: ['spec_approved']
  },
  design: {
    requiredFields: ['status', 'design_doc', 'draft_ready', 'finalized', 'review_completed'],
    historyEvents: ['design_draft_ready', 'design_review_complete', 'design_finalized']
  },
  plan: {
    requiredFields: ['status', 'plan_file', 'approved'],
    historyEvents: ['plan_created', 'plan_approved']
  },
  implement: {
    requiredFields: ['status'],
    historyEvents: ['implementation_complete']
  },
  pr: {
    requiredFields: ['status', 'pr_url', 'pr_number', 'ci_passed'],
    historyEvents: ['pre_flight_checks_passed', 'pr_created']
  },
  operational: {
    requiredFields: ['status'],
    historyEvents: ['operational_started']
  },
  execute: {
    requiredFields: ['status'],
    historyEvents: ['execution_complete']
  },
  document: {
    requiredFields: ['status'],
    historyEvents: ['documentation_complete']
  }
};

export default function (pi: ExtensionAPI) {
  pi.registerCommand("task:orchestrate", {
    description: "Complete Task Lifecycle Management - Orchestrate CLO-XX workflows (Loker / Linear)",
    handler: async (args: string, ctx: any) => {
      const argsList = args.split(/\s+/).filter(Boolean);
      const taskId = argsList.find(a => a.toLowerCase().startsWith("clo-"));
      const flags = new Set(argsList.filter(a => a.startsWith("--")));

      if (flags.has("--status")) {
        if (!taskId) {
          ctx.ui.notify("Please provide a task ID for status check", "error");
          return;
        }
        if (!validateTaskId(taskId)) {
          ctx.ui.notify(`Invalid task ID format: ${taskId}. Must match CLO-XX pattern.`, "error");
          return;
        }
        await showStatus(pi, taskId, ctx);
        return;
      }

      if (!taskId) {
        ctx.ui.notify("Usage: /task:orchestrate CLO-XX [--status] [--ops] [--spec] [--skip-discovery]", "error");
        return;
      }

      if (!validateTaskId(taskId)) {
        ctx.ui.notify(`Invalid task ID format: ${taskId}. Must match CLO-XX pattern.`, "error");
        return;
      }

      const workspaceRoot = process.cwd();
      const statePath = getWorkflowPath(workspaceRoot, taskId);

      let state: WorkflowState = {};
      if (fs.existsSync(statePath)) {
        const fileContent = fs.readFileSync(statePath, 'utf8');
        state = (yaml.load(fileContent) as WorkflowState) || {};
      }

      if (!state.workflow) {
        state = initializeWorkflow(taskId, flags);
        saveState(statePath, state);
      }

      let phaseModified = false;
      if (flags.has("--ops")) {
        if (state.task_type !== "operational" || state.workflow!.current_phase !== "operational") {
          state.task_type = "operational";
          state.workflow!.current_phase = "operational";
          addHistoryEvent(state, "workflow_modified", state.workflow!.current_phase || "unknown", "Switched to operational workflow via --ops flag");
          phaseModified = true;
        }
      } else if (flags.has("--spec")) {
        if (state.task_type !== "specification" || state.workflow!.current_phase !== "spec") {
          state.task_type = "specification";
          state.workflow!.current_phase = "spec";
          addHistoryEvent(state, "workflow_modified", state.workflow!.current_phase || "unknown", "Switched to spec workflow via --spec flag");
          phaseModified = true;
        }
      }

      if (flags.has("--skip-discovery")) {
        const currentPhase = state.workflow?.current_phase;
        if (currentPhase === "init" || currentPhase === "discovery") {
          if (!state.phases) state.phases = {};
          state.phases.discovery = { status: "skipped", approved: true, skip_reason: "--skip-discovery flag" };
          state.workflow!.current_phase = "design";
          addHistoryEvent(state, "discovery_skipped", "discovery", "Skipped via --skip-discovery flag");
          addHistoryEvent(state, "discovery_approved", "discovery", "Auto-approved skip");
          phaseModified = true;
        }
      }

      if (phaseModified) {
        saveState(statePath, state);
      }

      const validation = validatePhase(state);
      if (!validation.valid) {
        ctx.ui.notify(`Validation Failed: ${validation.errors.join(", ")}`, "error");
      }

      const currentPhase = state.workflow?.current_phase || "init";
      await dispatchPhase(pi, taskId, currentPhase, state, statePath, workspaceRoot);
    }
  });

  pi.registerTool({
    name: "update_workflow_state",
    label: "Update Workflow State",
    description: "Safely update the CLO workflow YAML with new state. Supports phase, workflow, linear, and root-level updates.",
    parameters: {
      type: "object",
      properties: {
        task_id: { type: "string", description: "Task ID (e.g. CLO-XX)" },
        phase: { type: "string", description: "Current phase being updated (required for history/action)" },
        action: { type: "string", description: "History action type (e.g. pre_flight_checks_passed, pr_created)" },
        details: { type: "string", description: "Details about the action" },
        field_updates: { type: "object", description: "DEPRECATED: Use phase_updates instead" },
        phase_updates: { type: "object", description: "Updates to apply to the specified phase (e.g. {status: 'complete'})" },
        workflow_updates: { type: "object", description: "Updates to apply to workflow root (e.g. {current_phase: 'design', status: 'active'})" },
        linear_updates: { type: "object", description: "Updates to apply to the linear block (e.g. {branch_actual: 'feat/clo-XX-...'})" },
        root_updates: { type: "object", description: "Updates to apply at the root level (e.g. {classification_reason: '...'})" }
      },
      required: ["task_id", "phase", "action", "details"]
    },
    execute: async (_toolCallId: string, params: any) => {
      if (!validateTaskId(params.task_id)) {
        return {
          content: [{ type: "text", text: `Invalid task ID format: ${params.task_id}` }],
          isError: true
        };
      }

      const workspaceRoot = process.cwd();
      const statePath = getWorkflowPath(workspaceRoot, params.task_id);

      let state: WorkflowState = {};
      if (fs.existsSync(statePath)) {
        const fileContent = fs.readFileSync(statePath, 'utf8');
        state = (yaml.load(fileContent) as WorkflowState) || {};
      }

      addHistoryEvent(state, params.action, params.phase, params.details);

      if (params.workflow_updates) {
        if (!state.workflow) state.workflow = {};
        const { current_phase, status, created_at, updated_at, ...rootFields } = params.workflow_updates;
        if (current_phase !== undefined) state.workflow.current_phase = current_phase;
        if (status !== undefined) state.workflow.status = status;
        if (created_at !== undefined) state.workflow.created_at = created_at;
        state.workflow.updated_at = updated_at !== undefined ? updated_at : new Date().toISOString();
        Object.assign(state, rootFields);
      } else {
        if (!state.workflow) state.workflow = {};
        state.workflow.updated_at = new Date().toISOString();
      }

      if (params.phase_updates) {
        const phase = params.phase;
        if (!state.phases) state.phases = {};
        if (!(state.phases as any)[phase]) (state.phases as any)[phase] = {};
        Object.assign((state.phases as any)[phase]!, params.phase_updates);
      }

      if (params.field_updates) {
        const phase = params.phase;
        if (!state.phases) state.phases = {};
        if (!(state.phases as any)[phase]) (state.phases as any)[phase] = {};
        Object.assign((state.phases as any)[phase]!, params.field_updates);
      }

      if (params.linear_updates) {
        if (!state.linear) state.linear = {};
        deepMerge(state.linear, params.linear_updates);
      }

      if (params.root_updates) {
        deepMerge(state, params.root_updates);
      }

      await saveStateSerialized(statePath, state);

      return {
        content: [{ type: "text", text: `Workflow state updated: ${params.action}` }],
        details: { task_id: params.task_id, action: params.action }
      };
    }
  });

  pi.registerTool({
    name: "transition_phase",
    label: "Transition Phase",
    description: "Transition workflow to the next phase with strict state machine validation (Loker rules)",
    parameters: {
      type: "object",
      properties: {
        task_id: { type: "string", description: "Task ID (e.g. CLO-XX)" },
        from_phase: { type: "string", description: "Current phase" },
        to_phase: { type: "string", description: "Next phase to transition to" },
        validation_override: { type: "boolean", description: "Skip validation (use with caution)", default: false }
      },
      required: ["task_id", "from_phase", "to_phase"]
    },
    execute: async (_toolCallId: string, params: any) => {
      if (!validateTaskId(params.task_id)) {
        return {
          content: [{ type: "text", text: `Invalid task ID format: ${params.task_id}` }],
          isError: true
        };
      }

      const workspaceRoot = process.cwd();
      const statePath = getWorkflowPath(workspaceRoot, params.task_id);

      let state: WorkflowState = {};
      if (fs.existsSync(statePath)) {
        const fileContent = fs.readFileSync(statePath, 'utf8');
        state = (yaml.load(fileContent) as WorkflowState) || {};
      }

      if (!state.workflow) {
        return {
          content: [{ type: "text", text: `No workflow found for ${params.task_id}` }],
          isError: true
        };
      }

      if (!params.validation_override) {
        const validation = validatePhaseTransition(state, params.from_phase, params.to_phase);
        if (!validation.valid) {
          return {
            content: [{ type: "text", text: `Transition blocked: ${validation.errors.join("\n")}` }],
            details: { blocked: true, errors: validation.errors },
            isError: true
          };
        }
      }

      state.workflow.current_phase = params.to_phase;
      state.workflow.updated_at = new Date().toISOString();
      addHistoryEvent(state, "phase_transition", params.from_phase, `Transitioned from ${params.from_phase} to ${params.to_phase}`);
      await saveStateSerialized(statePath, state);

      return {
        content: [{ type: "text", text: `Transitioned to ${params.to_phase} phase` }],
        details: { new_phase: params.to_phase }
      };
    }
  });
}

function validateTaskId(taskId: string): boolean {
  return TASK_ID_REGEX.test(taskId);
}

function getWorkflowPath(workspaceRoot: string, taskId: string): string {
  const normalizedId = taskId.toLowerCase().replace(/^clo-/, '');
  if (!/^\d+$/.test(normalizedId)) {
    throw new Error(`Invalid task ID format: ${taskId}`);
  }
  return path.join(workspaceRoot, `docs/status/clo-${normalizedId}-workflow.yaml`);
}

function initializeWorkflow(taskId: string, flags: Set<string>): WorkflowState {
  let taskType: 'development' | 'specification' | 'operational' = 'development';
  let initialPhase = 'init';

  if (flags.has("--ops")) {
    taskType = 'operational';
    initialPhase = 'operational';
  } else if (flags.has("--spec")) {
    taskType = 'specification';
    initialPhase = 'spec';
  }

  const now = new Date().toISOString();

  let phases: WorkflowState['phases'] = {};
  if (taskType === 'development') {
    phases = {
      discovery: { status: 'pending' },
      design: { status: 'pending' },
      plan: { status: 'pending' },
      implement: { status: 'pending' },
      pr: { status: 'pending' },
      complete: { status: 'pending' },
    };
  } else if (taskType === 'specification') {
    phases = {
      discovery: { status: 'skipped', skip_reason: 'Specification task', approved: true },
      spec: { status: 'pending' },
      design: { status: 'skipped', skip_reason: 'Specification task - using /spec instead of full design doc' },
      plan: { status: 'skipped', skip_reason: 'Specification task - spec decomposition is the plan' },
      implement: { status: 'pending' },
      pr: { status: 'pending' },
      complete: { status: 'pending' },
    };
  } else if (taskType === 'operational') {
    phases = {
      execute: { status: 'pending' },
      document: { status: 'pending' },
      complete: { status: 'pending' },
    };
  }

  return {
    task_id: taskId,
    task_type: taskType,
    classification_reason: '',
    task_profile: {
      has_backend: false,
      has_frontend: false,
      has_data_model: false,
      has_external_deps: false,
      skip_probe: false,
    },
    pending_human_action: null,
    linear: {
      team: 'Cloud-ai',
      project: 'Loker',
      status_at_start: 'Backlog',
      blocks: [],
      blocked_by: [],
    },
    workflow: {
      current_phase: initialPhase,
      status: "active",
      created_at: now,
      updated_at: now,
    },
    phases,
    history: [{
      timestamp: now,
      action: "workflow_started",
      phase: "init",
      details: `Workflow initialized for ${taskId} as ${taskType}`
    }]
  };
}

function validatePhase(state: WorkflowState): { valid: boolean; errors: string[] } {
  const currentPhase = state.workflow?.current_phase;
  if (!currentPhase) return { valid: false, errors: ["No current_phase set"] };
  if (currentPhase === "init" || currentPhase === "complete") return { valid: true, errors: [] };

  const config = PHASE_CONFIG[currentPhase];
  if (!config) return { valid: true, errors: [] };

  const errors: string[] = [];
  const phaseData = state.phases?.[currentPhase as keyof typeof state.phases];

  for (const field of config.requiredFields) {
    const value = phaseData?.[field as keyof typeof phaseData];
    if (value === undefined || value === null || value === "") {
      errors.push(`Missing required field: ${currentPhase}.${field}`);
    }
  }

  if ((phaseData as any)?.status === "complete") {
    const historyActions = new Set(state.history?.map(h => h.action) || []);
    for (const event of config.historyEvents) {
      if (!historyActions.has(event)) {
        errors.push(`Missing required history event: ${event}`);
      }
    }
  }

  return { valid: errors.length === 0, errors };
}

function validatePhaseTransition(state: WorkflowState, from: string, to: string): { valid: boolean; errors: string[] } {
  const errors: string[] = [];
  const currentPhase = state.workflow?.current_phase;

  if (from !== currentPhase) {
    errors.push(`Current phase mismatch: workflow is at "${currentPhase}", but attempting to transition from "${from}"`);
  }

  const allowedNext = ALLOWED_TRANSITIONS[from] || [];
  if (!allowedNext.includes(to)) {
    errors.push(`Invalid transition: cannot move from "${from}" to "${to}". Allowed next phases: ${allowedNext.join(', ') || 'none'}`);
  }

  const taskType = state.task_type || 'development';
  const allowedPhases = TYPE_ALLOWED_PHASES[taskType];
  if (allowedPhases && !allowedPhases.has(to)) {
    errors.push(`Phase "${to}" is not valid for task type "${taskType}"`);
  }

  const fromConfig = PHASE_CONFIG[from];
  if (fromConfig) {
    const phaseData = state.phases?.[from as keyof typeof state.phases];
    const phaseStatus = (phaseData as any)?.status;
    if (phaseStatus !== "complete" && phaseStatus !== "skipped") {
      errors.push(`Phase "${from}" not complete: status is "${phaseStatus || 'undefined'}", expected "complete" or "skipped"`);
    }

    if (phaseStatus !== "skipped") {
      for (const field of fromConfig.requiredFields) {
        const value = phaseData?.[field as keyof typeof phaseData];
        if (value === undefined || value === null || value === "") {
          errors.push(`Phase "${from}" missing required field: ${field}`);
        }
      }

      const historyActions = new Set(state.history?.map(h => h.action) || []);
      for (const event of fromConfig.historyEvents) {
        if (!historyActions.has(event)) {
          errors.push(`Phase "${from}" missing required history event: ${event}`);
        }
      }
    }
  }

  return { valid: errors.length === 0, errors };
}

function addHistoryEvent(state: WorkflowState, action: string, phase: string, details: string) {
  if (!state.history) state.history = [];
  state.history.push({
    timestamp: new Date().toISOString(),
    action,
    phase,
    details
  });
}

function deepMerge(target: any, source: any): any {
  for (const key of Object.keys(source)) {
    if (Array.isArray(target[key]) && Array.isArray(source[key])) {
      target[key] = target[key].concat(source[key]);
    } else if (
      source[key] && typeof source[key] === 'object' && !Array.isArray(source[key]) &&
      target[key] && typeof target[key] === 'object' && !Array.isArray(target[key])
    ) {
      deepMerge(target[key], source[key]);
    } else {
      target[key] = source[key];
    }
  }
  return target;
}

const writeLocks = new Map<string, Promise<void>>();

function saveState(statePath: string, state: WorkflowState) {
  const dir = path.dirname(statePath);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
  fs.writeFileSync(statePath, yaml.dump(state, { lineWidth: 120 }), 'utf8');
}

async function saveStateSerialized(statePath: string, state: WorkflowState): Promise<void> {
  const pending = writeLocks.get(statePath) || Promise.resolve();
  const next = pending.then(() => saveState(statePath, state));
  writeLocks.set(statePath, next.catch(() => {}));
  return next;
}

async function showStatus(pi: ExtensionAPI, taskId: string, ctx: any) {
  if (!validateTaskId(taskId)) {
    ctx.ui.notify(`Invalid task ID format: ${taskId}`, "error");
    return;
  }

  const workspaceRoot = process.cwd();
  const statePath = getWorkflowPath(workspaceRoot, taskId);

  if (!fs.existsSync(statePath)) {
    ctx.ui.notify(`No workflow file found for ${taskId}`, "warning");
    return;
  }

  const fileContent = fs.readFileSync(statePath, 'utf8');
  const state = yaml.load(fileContent) as WorkflowState;

  await dispatchPhase(pi, taskId, "status", state, statePath, workspaceRoot);
}

async function dispatchPhase(pi: ExtensionAPI, taskId: string, phase: string, state: WorkflowState, statePath: string, workspaceRoot: string) {
  const phaseFilePath = path.join(workspaceRoot, `.pi/orchestrator/phases/${phase}.md`);

  if (!fs.existsSync(phaseFilePath)) {
    pi.sendUserMessage(
      `Phase file not found: ${phase}.md. Please create it and try again.`,
      { deliverAs: "followUp" }
    );
    return;
  }

  const phaseInstructions = fs.readFileSync(phaseFilePath, 'utf8');

  const prompt = `
You are executing the Loker task orchestrator for ${taskId}.

Current State:
- Task ID: ${taskId}
- Current Phase: ${phase}
- Task Type: ${state.task_type || "unknown"}
- Workflow Status: ${state.workflow?.status || "unknown"}
- Linear Project: ${state.linear?.project || "Loker"} / Team: ${state.linear?.team || "Cloud-ai"}

State File: ${statePath}

Schema parity: This YAML must remain 100% compatible with the Claude flow at
.claude/commands/task/orchestrate.md so a task started in Claude can resume in pi
(and vice-versa). Do not invent new top-level keys; reuse existing ones.

Instructions for this phase:
---
${phaseInstructions}
---

Tools available:
- update_workflow_state: Update the workflow YAML with history events and field changes
- transition_phase: Move to the next phase (with validation)

For Linear interactions use the mcp__linear__* tools (served by the .pi/extensions/linear bridge).
Always use real newlines in Linear MCP body fields (no literal \\n).

Begin executing these instructions now. Update state after every significant action.
  `.trim();

  pi.sendUserMessage(prompt, { deliverAs: "followUp" });
}
