import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { SSEClientTransport } from '@modelcontextprotocol/sdk/client/sse.js';
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js';

type ExtensionAPI = any;

const LINEAR_MCP_SSE_URL = 'https://mcp.linear.app/sse';
const LINEAR_MCP_HTTP_URL = 'https://mcp.linear.app/mcp';
const TOOL_PREFIX = 'mcp__linear__';

export default function (pi: ExtensionAPI) {
  const apiKey = process.env.LINEAR_API_KEY;
  if (!apiKey) {
    pi.sendUserMessage(
      'LINEAR_API_KEY not set. Linear tools unavailable. Set it: export LINEAR_API_KEY=lin_api_...',
      { deliverAs: 'followUp' }
    );
    return;
  }

  registerLinearTools(pi, apiKey).catch((err: Error) => {
    pi.sendUserMessage(`Linear MCP connection failed: ${err.message}`, { deliverAs: 'followUp' });
  });
}

async function registerLinearTools(pi: ExtensionAPI, apiKey: string): Promise<void> {
  const transportMode = (process.env.LINEAR_MCP_TRANSPORT || 'http').toLowerCase();
  const headers = { Authorization: `Bearer ${apiKey}` };

  const transport =
    transportMode === 'sse'
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

  const client = new Client({ name: 'pi-linear', version: '1.0.0' });
  await Promise.race([
    client.connect(transport),
    new Promise<never>((_, reject) =>
      setTimeout(() => reject(new Error('Linear MCP connection timed out after 10s')), 10_000)
    ),
  ]);

  const { tools } = await client.listTools();
  if (!tools || tools.length === 0) {
    pi.sendUserMessage('Linear MCP: connected but no tools discovered.', { deliverAs: 'followUp' });
    return;
  }

  for (const tool of tools) {
    pi.registerTool({
      name: `${TOOL_PREFIX}${tool.name}`,
      label: tool.name,
      description: tool.description || `Linear: ${tool.name}`,
      parameters: tool.inputSchema || { type: 'object', properties: {} },
      async execute(_toolCallId: string, params: Record<string, unknown>) {
        const result = await client.callTool({ name: tool.name, arguments: params });

        const text =
          (result.content as Array<{ type: string; text?: string }>)
            ?.map((c) => (c.type === 'text' && c.text != null ? c.text : JSON.stringify(c)))
            .join('\n') || JSON.stringify(result);

        return {
          content: [{ type: 'text', text }],
          isError: Boolean(result.isError),
        };
      },
    });
  }

  pi.sendUserMessage(`Linear MCP: ${tools.length} tools registered.`, { deliverAs: 'followUp' });
}
