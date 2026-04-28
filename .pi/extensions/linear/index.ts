import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { SSEClientTransport } from "@modelcontextprotocol/sdk/client/sse.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";
import type { ExtensionAPI } from "@mariozechner/pi-coding-agent";
import { Type } from "typebox";
import { Text } from "@mariozechner/pi-tui";

const LINEAR_MCP_SSE_URL = "https://mcp.linear.app/sse";
const LINEAR_MCP_HTTP_URL = "https://mcp.linear.app/mcp";
const TOOL_PREFIX = "mcp__linear__";

// Approved tool subset — see docs/guides/linear-mcp-adapter.md §2.
const APPROVED_TOOLS = new Set<string>([
  "list_issues",
  "get_issue",
  "save_issue",
  "list_comments",
  "save_comment",
  "list_issue_statuses",
  "list_projects",
]);

// Conditional tools registered alongside the core 7. See adapter §2.2.
const CONDITIONAL_TOOLS = new Set<string>(["get_team"]);

const registeredLinearTools = new Set<string>();

type MCPTool = {
  name: string;
  description?: string;
  inputSchema?: unknown;
};

export default function (pi: ExtensionAPI) {
  const apiKey = process.env.LINEAR_API_KEY;
  if (!apiKey) {
    pi.sendUserMessage(
      "LINEAR_API_KEY not set. Linear tools unavailable. Set it: export LINEAR_API_KEY=lin_api_...",
      { deliverAs: "followUp" },
    );
    return;
  }

  // Register once at startup.
  registerLinearTools(pi, apiKey).catch((err: Error) => {
    pi.sendUserMessage(`Linear MCP connection failed: ${err.message}`, { deliverAs: "followUp" });
  });

  // Re-run discovery on session start / reload to catch new/changed tools.
  pi.on("session_start", async () => {
    const connected = await registerLinearTools(pi, apiKey).catch((err: Error) => {
      pi.sendUserMessage(`Linear MCP refresh failed: ${err.message}`, { deliverAs: "followUp" });
      return false;
    });

    if (!connected) {
      pi.sendUserMessage("Linear MCP unavailable. Keeping previously registered tools.", { deliverAs: "followUp" });
    }
  });
}


function schemaToTypeBox(schema: unknown): unknown {
  if (!schema || typeof schema !== "object") {
    return Type.Object({});
  }

  const input = schema as {
    type?: string;
    properties?: Record<string, unknown>;
    required?: string[];
    items?: unknown;
    enum?: unknown[];
    additionalProperties?: unknown;
    description?: string;
  };

  switch (input.type) {
    case "string":
      return Type.String({ description: input.description });
    case "number":
      return Type.Number({ description: input.description });
    case "integer":
      return Type.Integer({ description: input.description });
    case "boolean":
      return Type.Boolean({ description: input.description });
    case "array":
      return Type.Array(schemaToTypeBox(input.items), { description: input.description });
    case "object": {
      const required = new Set(input.required || []);
      const entries = input.properties || {};
      const props: Record<string, unknown> = {};

      for (const [key, value] of Object.entries(entries)) {
        const child = schemaToTypeBox(value);
        props[key] = required.has(key) ? child : Type.Optional(child as any);
      }

      return Object.keys(props).length > 0
        ? Type.Object(props, {
            additionalProperties: false,
            description: input.description,
          })
        : Type.Object({}, { additionalProperties: false, description: input.description });
    }
    default:
      if (Array.isArray(input.enum)) {
        const variants = input.enum
          .filter((item) => typeof item === "string")
          .map((item) => Type.Literal(item));
        if (variants.length > 0) {
          return Type.Union(variants);
        }
      }
      return Type.Record(Type.String(), Type.Any());
  }
}

async function registerLinearTools(pi: ExtensionAPI, apiKey: string): Promise<boolean> {
  const transportMode = (process.env.LINEAR_MCP_TRANSPORT || "http").toLowerCase();
  const headers = { Authorization: `Bearer ${apiKey}` };

  const transport =
    transportMode === "sse"
      ? new SSEClientTransport(new URL(LINEAR_MCP_SSE_URL), {
          requestInit: { headers },
          eventSourceInit: {
            fetch: (url: string | URL, init?: RequestInit) =>
              fetch(url, { ...(init || {}), headers: { ...(init?.headers || {}), ...headers } }),
          },
        })
      : new StreamableHTTPClientTransport(new URL(LINEAR_MCP_HTTP_URL), {
          requestInit: { headers },
        });

  const client = new Client({ name: "pi-linear", version: "1.0.0" });

  await Promise.race([
    client.connect(transport),
    new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error("Linear MCP connection timed out after 10s")), 10_000),
    ),
  ]);

  const { tools } = await client.listTools();
  if (!tools || tools.length === 0) {
    pi.sendUserMessage("Linear MCP: connected but no tools discovered.", { deliverAs: "followUp" });
    return false;
  }

  const discoveredCount = Array.isArray(tools) ? tools.length : 0;
  const fullSurface = process.env.LINEAR_MCP_FULL_SURFACE === "1";
  const candidates = fullSurface
    ? (tools as MCPTool[])
    : (tools as MCPTool[]).filter((t) => APPROVED_TOOLS.has(t.name) || CONDITIONAL_TOOLS.has(t.name));

  for (const tool of candidates) {
    const toolName = `${TOOL_PREFIX}${tool.name}`;
    if (registeredLinearTools.has(toolName)) {
      continue;
    }
    registeredLinearTools.add(toolName);

    const params = tool.inputSchema ? schemaToTypeBox(tool.inputSchema) : Type.Object({});
    const description = tool.description || `Linear: ${tool.name}`;

    pi.registerTool({
      name: toolName,
      label: tool.name,
      description,
      promptSnippet: `Use ${toolName} when interacting with Linear via MCP.`,
      promptGuidelines: [`Use ${toolName} to execute Linear MCP action ${tool.name}.`],
      parameters: params,

      async execute(_toolCallId: string, params: Record<string, unknown>) {
        const result = await client.callTool({ name: tool.name, arguments: params });
        const resultText =
          (result.content as Array<{ type: string; text?: string }> | undefined)
            ?.map((chunk) => (chunk.type === "text" && chunk.text != null ? chunk.text : JSON.stringify(chunk)))
            .join("\n") || JSON.stringify(result);

        if (result.isError) {
          throw new Error(`Linear tool ${tool.name} failed: ${resultText}`);
        }

        return {
          content: [{ type: "text", text: resultText }],
          details: {
            linear_tool: tool.name,
            result_count: Array.isArray(result.content) ? result.content.length : 0,
          },
        };
      },

      renderCall(args: Record<string, unknown>, theme: any, _ctx: any) {
        const argCount = args ? Object.keys(args).length : 0;
        return new Text(
          `${theme.fg("toolTitle", theme.bold(tool.name))} ${theme.fg("muted", `${argCount} args`)}`,
          0,
          0,
        );
      },

      renderResult(result: any, _opts: any, theme: any, _ctx: any) {
        if (result.isError) {
          return new Text(theme.fg("error", `Linear tool failed`), 0, 0);
        }
        const details = result.content?.[0];
        const text = typeof details?.text === "string" ? details.text : "no output";
        return new Text(theme.fg("success", text.slice(0, 120)), 0, 0);
      },
    });
  }

  const surfaceMode = fullSurface ? "full" : "approved-subset";
  pi.sendUserMessage(
    `Linear MCP: registered ${registeredLinearTools.size}/${discoveredCount} tools (${surfaceMode}).`,
    { deliverAs: "followUp" },
  );
  return true;
}
