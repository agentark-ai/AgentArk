import { test, expect } from "@playwright/test";

test.describe("Chat Activity UI @smoke", () => {
  test.beforeEach(async ({ page }) => {
    await page.addInitScript(() => {
      window.localStorage.setItem("agentark.tour.completed", "1");
    });
  });

  test("sending first message in a draft chat does not blank the UI", async ({ page }) => {
    let createdConversationId = "";
    let userMessage = "";
    const assistantMessage = "Saved your preference and kept the chat UI stable.";
    const pageErrors: string[] = [];

    page.on("pageerror", (error) => {
      pageErrors.push(String(error));
    });

    await page.route("**/conversations*", async (route) => {
      if (route.request().method().toUpperCase() === "POST") {
        createdConversationId = "conv-pref-ui";
        await route.fulfill({
          contentType: "application/json",
          body: JSON.stringify({
            id: createdConversationId,
            title: "Preference capture chat",
            channel: "web",
            created_at: "2026-03-08T13:00:00.000Z",
            updated_at: "2026-03-08T13:00:00.000Z",
            message_count: 0
          })
        });
        return;
      }
      const conversations = createdConversationId
        ? [
            {
              id: createdConversationId,
              title: "Preference capture chat",
              channel: "web",
              created_at: "2026-03-08T13:00:00.000Z",
              updated_at: "2026-03-08T13:00:08.000Z",
              message_count: 2,
              archived: false
            }
          ]
        : [];
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations,
          total: conversations.length,
          limit: 30,
          offset: 0
        })
      });
    });

    await page.route("**/conversations/*/messages?**", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages:
            conversationId && conversationId === createdConversationId
              ? [
                  {
                    id: "msg-user-pref",
                    role: "user",
                    content: userMessage,
                    timestamp: "2026-03-08T13:00:01.000Z",
                    model_used: null,
                    trace_id: null
                  },
                  {
                    id: "msg-assistant-pref",
                    role: "assistant",
                    content: assistantMessage,
                    timestamp: "2026-03-08T13:00:08.000Z",
                    model_used: "test-model",
                    trace_id: null
                  }
                ]
              : []
        })
      });
    });

    await page.route("**/conversations/*", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      if (!conversationId || conversationId !== createdConversationId) {
        await route.fulfill({
          status: 404,
          contentType: "application/json",
          body: JSON.stringify({ error: "not found" })
        });
        return;
      }
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: createdConversationId,
          title: "Preference capture chat",
          channel: "web",
          created_at: "2026-03-08T13:00:00.000Z",
          updated_at: "2026-03-08T13:00:08.000Z",
          message_count: 2
        })
      });
    });

    await page.route("**/chat/stream", async (route) => {
      const payload = route.request().postDataJSON() as {
        conversation_id?: string;
        message?: string;
      };
      createdConversationId = payload.conversation_id || "conv-pref-ui";
      userMessage = payload.message || "i love samsung and hate apple";
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: [
          'event: thinking\ndata: {"title":"Message Received","detail":"Channel: web | Length: 31 chars","step_type":"info"}\n\n',
          'event: content\ndata: {"conversation_id":"' +
            createdConversationId +
            '","content":"' +
            assistantMessage +
            '"}\n\n',
          "event: done\ndata: {}\n\n"
        ].join("")
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    const input = page.locator("textarea[aria-label='Message']").first();
    await expect(input).toBeVisible({ timeout: 10_000 });

    await input.fill("i love samsung and hate apple");
    await input.press("Enter");

    await expect(page.locator("text=Preference capture chat")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("text=Saved your preference and kept the chat UI stable.")).toBeVisible({
      timeout: 10_000
    });
    await expect(input).toBeVisible();
    expect(pageErrors).toEqual([]);
  });

  test("renders streamed tool access as compact inline chat activity", async ({ page }) => {
    let createdConversationId = "";
    let userMessage = "";
    const assistantMessage = "I found three fresh messages in your inbox.";
    const rawToolName = "vendor_mail_connector_v2";

    await page.addInitScript(() => {
      window.sessionStorage.removeItem("agentark.chat.lastConversationId");
      window.sessionStorage.removeItem("agentark.chat.pendingRun");
      window.sessionStorage.removeItem("agentark.chat.draftMode");
    });

    await page.route("**/conversations?**", async (route) => {
      const conversations = createdConversationId
        ? [
            {
              id: createdConversationId,
              title: "Inbox check",
              channel: "web",
              created_at: "2026-05-24T09:00:00.000Z",
              updated_at: "2026-05-24T09:00:08.000Z",
              message_count: 2,
              archived: false
            }
          ]
        : [];
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations,
          total: conversations.length,
          limit: 30,
          offset: 0
        })
      });
    });

    await page.route("**/conversations/*/messages?**", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages:
            conversationId && conversationId === createdConversationId
              ? [
                  {
                    id: "msg-user-inbox-check",
                    role: "user",
                    content: userMessage,
                    timestamp: "2026-05-24T09:00:01.000Z",
                    model_used: null,
                    trace_id: null
                  },
                  {
                    id: "msg-assistant-inbox-check",
                    role: "assistant",
                    content: assistantMessage,
                    timestamp: "2026-05-24T09:00:08.000Z",
                    model_used: "test-model",
                    trace_id: null
                  }
                ]
              : []
        })
      });
    });

    await page.route("**/conversations/*", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      if (!conversationId || conversationId !== createdConversationId) {
        await route.fulfill({
          status: 404,
          contentType: "application/json",
          body: JSON.stringify({ error: "not found" })
        });
        return;
      }
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: createdConversationId,
          title: "Inbox check",
          channel: "web",
          created_at: "2026-05-24T09:00:00.000Z",
          updated_at: "2026-05-24T09:00:08.000Z",
          message_count: 2
        })
      });
    });

    await page.route("**/chat/stream", async (route) => {
      const payload = route.request().postDataJSON() as {
        conversation_id?: string;
        message?: string;
      };
      createdConversationId = payload.conversation_id || "conv-inbox-access";
      userMessage = payload.message || "check my new emails";
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: [
          `event: tool_start\ndata: {"name":"${rawToolName}","label":"Accessing Gmail account","activity_detail":"Checking messages","account":"primary"}\n\n`,
          `event: tool_result\ndata: {"name":"${rawToolName}","label":"Accessing Gmail account","activity_detail":"Inbox checked","content":"3 messages returned","status":"ok","count":3}\n\n`,
          `event: content\ndata: {"conversation_id":"${createdConversationId}","content":"${assistantMessage}"}\n\n`,
          "event: done\ndata: {}\n\n"
        ].join("")
      });
    });

    await page.route("**/conversations", async (route) => {
      if (route.request().method().toUpperCase() !== "POST") {
        await route.fallback();
        return;
      }
      createdConversationId = "conv-inbox-access";
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: createdConversationId,
          title: "Inbox check",
          channel: "web",
          created_at: "2026-05-24T09:00:00.000Z",
          updated_at: "2026-05-24T09:00:00.000Z",
          message_count: 0
        })
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    const input = page.locator("textarea[aria-label='Message']").first();
    await expect(input).toBeVisible({ timeout: 10_000 });

    await input.fill("check my new emails");
    await input.press("Enter");

    const activityRow = page
      .locator(".chat-transcript-action-row")
      .filter({ hasText: "Accessing Gmail account" })
      .first();
    await expect(activityRow).toBeVisible({ timeout: 10_000 });
    await expect(activityRow.locator(".chat-transcript-action-status")).toContainText(
      "success"
    );
    await expect(page.locator(`text=${assistantMessage}`)).toBeVisible({
      timeout: 10_000
    });
    await expect(page.locator(".chat-thread")).not.toContainText(rawToolName);
  });

  test("renders summarized trace activity without raw dumps", async ({ page }) => {
    const conversationId = "conv-framework-regression";
    const traceId = "trace-framework-regression";
    const createdAt = "2026-03-08T17:20:00.000Z";
    const updatedAt = "2026-03-08T17:22:00.000Z";
    const assistantMessage =
      "I checked the framework regression path and summarized the blocked step.";

    await page.route("**/conversations?**", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations: [
            {
              id: conversationId,
              title: "Framework regression chat",
              channel: "web",
              created_at: createdAt,
              updated_at: updatedAt,
              message_count: 2,
              archived: false
            }
          ],
          total: 1,
          limit: 20,
          offset: 0
        })
      });
    });

    await page.route(`**/conversations/${conversationId}`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: conversationId,
          title: "Framework regression chat",
          channel: "web",
          created_at: createdAt,
          updated_at: updatedAt,
          message_count: 2
        })
      });
    });

    await page.route(`**/conversations/${conversationId}/messages?**`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages: [
            {
              id: "msg-user-1",
              role: "user",
              content: "fix the framework so raw tool dumps never reach chat",
              timestamp: createdAt,
              model_used: null,
              trace_id: null
            },
            {
              id: "msg-assistant-1",
              role: "assistant",
              content: assistantMessage,
              timestamp: updatedAt,
              model_used: "z-ai/glm-5",
              trace_id: traceId
            }
          ]
        })
      });
    });

    await page.route(`**/trace/${traceId}`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: traceId,
          message: "fix the framework so raw tool dumps never reach chat",
          channel: "web",
          started_at: "2026-03-08 17:20:00",
          completed_at: "2026-03-08 17:22:00",
          duration_ms: 122000,
          response: assistantMessage,
          proof_id: "proof-test",
          steps: [
            {
              icon: "[wait]",
              title: "Still Working",
              detail: "Memory/context setup in progress. No new output yet.",
              step_type: "heartbeat",
              data: null,
              time: "17:20:05"
            },
            {
              icon: "[wait]",
              title: "Still Working",
              detail: "Memory/context setup in progress. No new output yet (15s idle).",
              step_type: "thinking",
              data: { idle_secs: 15 },
              time: "17:20:20"
            },
            {
              icon: "[ok]",
              title: "Tool finished: file_read",
              detail:
                "<!DOCTYPE html><html><head><title>arXiv Research Monitor | RL & Time-Series</title></head><body><div>demo</div></body></html>",
              step_type: "tool_result",
              data: null,
              time: "17:21:10"
            },
            {
              icon: "[ok]",
              title: "Tool finished: http_get",
              detail: "Tool 'Http_get' Blocked By Safety Policy",
              step_type: "tool_result",
              data: null,
              time: "17:21:35"
            }
          ]
        })
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    const conversationRow = page.locator("text=Framework regression chat").first();
    await expect(conversationRow).toBeVisible({ timeout: 10_000 });
    await conversationRow.click();

    const traceToggle = page.locator(".chat-inline-trace-toggle").first();
    await expect(traceToggle).toContainText("Now: Http Get Blocked", { timeout: 10_000 });

    await traceToggle.click();

    const htmlStep = page.locator(
      '.chat-inline-step[title*="Read HTML document: arXiv Research Monitor | RL & Time-Series."]'
    );
    await expect(htmlStep.first()).toBeVisible({ timeout: 10_000 });

    const blockedStep = page.locator(
      '.chat-inline-step[title*="Blocked by safety policy. The agent needs a different approach."]'
    );
    await expect(blockedStep.first()).toBeVisible({ timeout: 10_000 });

    await expect(page.locator("body")).not.toContainText("<!DOCTYPE html>");
    await expect(page.locator("body")).not.toContainText("Tool 'Http_get' Blocked By Safety Policy");
    await expect(page.locator("body")).not.toContainText("matched_app");
  });

  test("renders accidentally indented prose as prose, not a code preview", async ({ page }) => {
    const conversationId = "conv-indented-prose";
    const createdAt = "2026-05-03T16:56:00.000Z";
    const updatedAt = "2026-05-03T16:57:00.000Z";
    const assistantMessage = [
      "Here are things you can ask AgentArk to do:",
      "",
      '    "Search the web or internal docs for X."',
      '    "Run some Python/JS code and show me the result."',
      "    What is confirmed as available right now",
      "",
      "A real fenced code block should still render as code:",
      "",
      "```ts",
      "const answer = 42;",
      "```"
    ].join("\n");

    await page.route("**/conversations?**", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations: [
            {
              id: conversationId,
              title: "Indented prose chat",
              channel: "web",
              created_at: createdAt,
              updated_at: updatedAt,
              message_count: 2,
              archived: false
            }
          ],
          total: 1,
          limit: 20,
          offset: 0
        })
      });
    });

    await page.route(`**/conversations/${conversationId}`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: conversationId,
          title: "Indented prose chat",
          channel: "web",
          created_at: createdAt,
          updated_at: updatedAt,
          message_count: 2
        })
      });
    });

    await page.route(`**/conversations/${conversationId}/messages?**`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages: [
            {
              id: "msg-user-indented",
              role: "user",
              content: "show me a capability walkthrough",
              timestamp: createdAt,
              model_used: null,
              trace_id: null
            },
            {
              id: "msg-assistant-indented",
              role: "assistant",
              content: assistantMessage,
              timestamp: updatedAt,
              model_used: "test-model",
              trace_id: null
            }
          ]
        })
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    await page.getByRole("button", { name: /^Conversations$/ }).first().click();
    const conversationRow = page
      .locator(".conversation-card")
      .filter({ hasText: "Indented prose chat" })
      .first();
    await expect(conversationRow).toBeVisible({ timeout: 10_000 });
    await conversationRow.click();

    await expect(page.locator(".chat-markdown").first()).toContainText(
      "Search the web or internal docs for X",
      { timeout: 10_000 }
    );
    await page.waitForTimeout(700);
    await expect(page.locator(".chat-md-ide")).toHaveCount(1);
    await expect(page.locator(".chat-md-ide").first()).toContainText("main.ts");
    await expect(page.locator(".chat-md-ide").first()).not.toContainText(
      "Search the web or internal docs for X"
    );
  });

  test("keeps markdown tables in final assistant messages", async ({ page }) => {
    const conversationId = "conv-final-table";
    const createdAt = "2026-05-03T17:00:00.000Z";
    const updatedAt = "2026-05-03T17:01:00.000Z";
    const assistantMessage = [
      "Based on the capability registry results:",
      "",
      "| Area | What Ark Evolve learns |",
      "| --- | --- |",
      "| Routing | Which model/provider works best for a task type. |",
      "| Tools | Which action sequence succeeds most often. |",
      "",
      "- Recurring workflows",
      "- User correction patterns"
    ].join("\n");

    await page.route("**/conversations?**", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations: [
            {
              id: conversationId,
              title: "Final table chat",
              channel: "web",
              created_at: createdAt,
              updated_at: updatedAt,
              message_count: 2,
              archived: false
            }
          ],
          total: 1,
          limit: 20,
          offset: 0
        })
      });
    });

    await page.route(`**/conversations/${conversationId}`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: conversationId,
          title: "Final table chat",
          channel: "web",
          created_at: createdAt,
          updated_at: updatedAt,
          message_count: 2
        })
      });
    });

    await page.route(`**/conversations/${conversationId}/messages?**`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages: [
            {
              id: "msg-user-table",
              role: "user",
              content: "explain ark evolve in detail",
              timestamp: createdAt,
              model_used: null,
              trace_id: null
            },
            {
              id: "msg-assistant-table",
              role: "assistant",
              content: assistantMessage,
              timestamp: updatedAt,
              model_used: "test-model",
              trace_id: null
            }
          ]
        })
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    await page.getByRole("button", { name: /^Conversations$/ }).first().click();
    const conversationRow = page
      .locator(".conversation-card")
      .filter({ hasText: "Final table chat" })
      .first();
    await expect(conversationRow).toBeVisible({ timeout: 10_000 });
    await conversationRow.click();

    await expect(page.locator(".chat-markdown table")).toHaveCount(1, {
      timeout: 10_000
    });
    await expect(page.locator(".chat-markdown table").first()).toContainText(
      "Routing"
    );
    await expect(page.locator(".chat-markdown-search-brief")).toHaveCount(0);
  });

  test("collapses raw activity payloads until the user expands them", async ({ page }) => {
    let createdConversationId = "";
    let userMessage = "";
    const assistantMessage = "I checked the file and kept the raw payload in the side panel.";
    const payloadJson = JSON.stringify({
      kind: "tool_dispatch",
      tool_name: "file_read",
      status: "running",
      path: "src/main.rs",
      run_id: "run-live-1",
      ts: "2026-03-09T11:30:00.000Z"
    });

    await page.setViewportSize({ width: 2100, height: 1200 });

    await page.route("**/conversations?**", async (route) => {
      const conversations = createdConversationId
        ? [
            {
              id: createdConversationId,
              title: "Payload disclosure chat",
              channel: "web",
              created_at: "2026-03-09T11:30:00.000Z",
              updated_at: "2026-03-09T11:30:08.000Z",
              message_count: 2,
              archived: false
            }
          ]
        : [];
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations,
          total: conversations.length,
          limit: 30,
          offset: 0
        })
      });
    });

    await page.route("**/conversations/*/messages?**", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages:
            conversationId && conversationId === createdConversationId
              ? [
                  {
                    id: "msg-user-payload",
                    role: "user",
                    content: userMessage,
                    timestamp: "2026-03-09T11:30:01.000Z",
                    model_used: null,
                    trace_id: null
                  },
                  {
                    id: "msg-assistant-payload",
                    role: "assistant",
                    content: assistantMessage,
                    timestamp: "2026-03-09T11:30:08.000Z",
                    model_used: "test-model",
                    trace_id: null
                  }
                ]
              : []
        })
      });
    });

    await page.route("**/conversations/*", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      if (!conversationId || conversationId !== createdConversationId) {
        await route.fulfill({
          status: 404,
          contentType: "application/json",
          body: JSON.stringify({ error: "not found" })
        });
        return;
      }
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: createdConversationId,
          title: "Payload disclosure chat",
          channel: "web",
          created_at: "2026-03-09T11:30:00.000Z",
          updated_at: "2026-03-09T11:30:08.000Z",
          message_count: 2
        })
      });
    });

    await page.route("**/chat/stream", async (route) => {
      const payload = route.request().postDataJSON() as {
        conversation_id?: string;
        message?: string;
      };
      createdConversationId = payload.conversation_id || "conv-payload-ui";
      userMessage = payload.message || "read src/main.rs and keep the raw payload collapsed";
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: [
          'event: thinking\ndata: {"title":"Message Received","detail":"Channel: web | Length: 48 chars","step_type":"info"}\n\n',
          `event: tool_progress\ndata: ${JSON.stringify({
            name: "file_read",
            content: payloadJson,
            kind: "tool_dispatch",
            tool_name: "file_read",
            status: "running",
            path: "src/main.rs",
            run_id: "run-live-1",
            ts: "2026-03-09T11:30:00.000Z"
          })}\n\n`,
          `event: content\ndata: {"conversation_id":"${createdConversationId}","content":"${assistantMessage}"}\n\n`,
          "event: done\ndata: {}\n\n"
        ].join("")
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    const input = page.locator("textarea[aria-label='Message']").first();
    await expect(input).toBeVisible({ timeout: 10_000 });
    await input.fill("read src/main.rs and keep the raw payload collapsed");
    await input.press("Enter");

    await expect(page.locator(".activity-row-summary").filter({ hasText: "File Read dispatch ready." }).first()).toBeVisible({
      timeout: 10_000
    });
    await expect(page.locator(".activity-payload-pre")).toHaveCount(0);
    await expect(page.locator("body")).not.toContainText('"run_id": "run-live-1"');

    await page.getByRole("button", { name: "Show data" }).first().click();

    const payloadPre = page.locator(".activity-payload-pre").first();
    await expect(payloadPre).toBeVisible({ timeout: 10_000 });
    await expect(payloadPre).toContainText('"run_id": "run-live-1"');
    await expect(payloadPre).toContainText('"path": "src/main.rs"');
  });

  test("shows a stopped run card and resumes in-thread without a duplicate pending user bubble", async ({ page }) => {
    const conversationId = "conv-resume-inline";
    const taskId = "task-resume-inline";
    const userMessage = "please keep going";
    const partialAssistant = "Partial answer before the run was stopped.";
    const resumedAssistant = "Finished answer after resuming in chat.";
    let resumed = false;

    await page.addInitScript(
      ({ conversationId, taskId, userMessage, partialAssistant }) => {
        window.sessionStorage.setItem(
          "agentark.chat.pendingRun",
          JSON.stringify({
            conversationId,
            message: userMessage,
            startedAt: Date.now(),
            mode: "fresh",
            phase: "interrupted",
            taskId,
            streamingResponse: partialAssistant,
            streamingSteps: [
              {
                title: "Tool started: file_read",
                detail: "Reading the workspace before the stop.",
                step_type: "tool_start"
              }
            ],
            failedUserMessage: ""
          })
        );
      },
      { conversationId, taskId, userMessage, partialAssistant }
    );

    await page.route("**/tasks?**", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ tasks: [] })
      });
    });

    await page.route("**/conversations?**", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations: [
            {
              id: conversationId,
              title: "Stopped run chat",
              channel: "web",
              created_at: "2026-03-31T01:00:00.000Z",
              updated_at: "2026-03-31T01:02:00.000Z",
              message_count: resumed ? 2 : 1,
              archived: false
            }
          ],
          total: 1,
          limit: 30,
          offset: 0
        })
      });
    });

    await page.route(`**/conversations/${conversationId}`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: conversationId,
          title: "Stopped run chat",
          channel: "web",
          created_at: "2026-03-31T01:00:00.000Z",
          updated_at: "2026-03-31T01:02:00.000Z",
          message_count: resumed ? 2 : 1
        })
      });
    });

    await page.route(`**/conversations/${conversationId}/messages?**`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages: resumed
            ? [
                {
                  id: "msg-user-resume-inline",
                  role: "user",
                  content: userMessage,
                  timestamp: "2026-03-31T01:00:01.000Z",
                  model_used: null,
                  trace_id: null
                },
                {
                  id: "msg-assistant-resume-inline",
                  role: "assistant",
                  content: resumedAssistant,
                  timestamp: "2026-03-31T01:02:10.000Z",
                  model_used: "test-model",
                  trace_id: null
                }
              ]
            : [
                {
                  id: "msg-user-resume-inline",
                  role: "user",
                  content: userMessage,
                  timestamp: "2026-03-31T01:00:01.000Z",
                  model_used: null,
                  trace_id: null
                }
              ]
        })
      });
    });

    await page.route(`**/tasks/${taskId}/resume-chat/stream`, async (route) => {
      resumed = true;
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: [
          `event: task_started\ndata: {"task_id":"${taskId}","description":"Resume stopped chat","status":"in_progress","work_type":"task","conversation_id":"${conversationId}"}\n\n`,
          `event: content\ndata: {"conversation_id":"${conversationId}","content":"${resumedAssistant}"}\n\n`,
          "event: done\ndata: {}\n\n"
        ].join("")
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    await expect(page.locator("text=AgentArk | stopped")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(`text=${partialAssistant}`)).toBeVisible({ timeout: 10_000 });

    await page.getByRole("button", { name: "Resume" }).click();

    await expect(page.locator("text=You | sending...")).toHaveCount(0);
    await expect(page.locator(`text=${resumedAssistant}`)).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("text=AgentArk | stopped")).toHaveCount(0);
  });

  test("shows live draft code and phase status in the workspace panel", async ({ page }) => {
    let createdConversationId = "";
    let userMessage = "";
    const assistantMessage = "Built the first draft and kept streaming the file into the workspace.";

    await page.setViewportSize({ width: 1440, height: 960 });

    await page.route("**/api/apps", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ apps: [] })
      });
    });

    await page.route("**/tunnel/status", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({})
      });
    });

    await page.route("**/conversations?**", async (route) => {
      const conversations = createdConversationId
        ? [
            {
              id: createdConversationId,
              title: "Live draft stream chat",
              channel: "web",
              created_at: "2026-03-31T12:00:00.000Z",
              updated_at: "2026-03-31T12:00:12.000Z",
              message_count: 2,
              archived: false
            }
          ]
        : [];
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations,
          total: conversations.length,
          limit: 20,
          offset: 0
        })
      });
    });

    await page.route("**/conversations/*/messages?**", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages:
            conversationId && conversationId === createdConversationId
              ? [
                  {
                    id: "msg-user-live-draft",
                    role: "user",
                    content: userMessage,
                    timestamp: "2026-03-31T12:00:01.000Z",
                    model_used: null,
                    trace_id: null
                  },
                  {
                    id: "msg-assistant-live-draft",
                    role: "assistant",
                    content: assistantMessage,
                    timestamp: "2026-03-31T12:00:12.000Z",
                    model_used: "test-model",
                    trace_id: null
                  }
                ]
              : []
        })
      });
    });

    await page.route("**/conversations/*", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      if (!conversationId || conversationId !== createdConversationId) {
        await route.fulfill({
          status: 404,
          contentType: "application/json",
          body: JSON.stringify({ error: "not found" })
        });
        return;
      }
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: createdConversationId,
          title: "Live draft stream chat",
          channel: "web",
          created_at: "2026-03-31T12:00:00.000Z",
          updated_at: "2026-03-31T12:00:12.000Z",
          message_count: 2
        })
      });
    });

    await page.route("**/chat/stream", async (route) => {
      const payload = route.request().postDataJSON() as {
        conversation_id?: string;
        message?: string;
      };
      createdConversationId = payload.conversation_id || "conv-live-draft";
      userMessage = payload.message || "build me a simple hello world app";
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: [
          'event: tool_progress\ndata: {"name":"agent_turn_loop","content":"Planning the app fix and preparing the app delivery action.","kind":"agent_loop_progress","phase":"model_call","focus":"app_delivery","title":"Calling model","stream_key":"agent-loop:model_call"}\n\n',
          'event: tool_progress\ndata: {"name":"app_deploy","content":"Drafting src/App.tsx","kind":"phase_status","phase":"generating_files","label":"Generating files","detail":"Drafting src/App.tsx","elapsed_secs":7,"stream_key":"phase-status:app_deploy:generating_files"}\n\n',
          'event: tool_progress\ndata: {"name":"app_deploy","content":"Drafting src/App.tsx","kind":"draft_file","file":"src/App.tsx","phase":"generating_files","stream_key":"draft-file:app_deploy:src/App.tsx","content_snapshot":"export default function App() {\\n  return <main>Hello world</main>;\\n}\\n","line":3,"total_lines":3,"done":false}\n\n',
          'event: tool_progress\ndata: {"name":"app_deploy","content":"Drafting src/App.tsx","kind":"draft_file","file":"src/App.tsx","phase":"generating_files","stream_key":"draft-file:app_deploy:src/App.tsx","content_snapshot":"export default function App() {\\n  return <main>Hello world</main>;\\n}\\n","line":3,"total_lines":3,"done":true}\n\n',
          'event: content\ndata: {"conversation_id":"' +
            createdConversationId +
            '","content":"' +
            assistantMessage +
            '"}\n\n',
          "event: done\ndata: {}\n\n"
        ].join("")
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    const input = page.locator("textarea[aria-label='Message']").first();
    await expect(input).toBeVisible({ timeout: 10_000 });

    await input.fill("build me a simple hello world app");
    await input.press("Enter");

    await expect(page.locator(".computer-pane-heading")).toContainText("AgentArk's Console", { timeout: 10_000 });
    const paneTabs = page.locator(".computer-pane-tabs").first();
    const filesTab = paneTabs.getByRole("tab", { name: "Files" });
    await expect(filesTab).toBeVisible({ timeout: 10_000 });
    await expect(filesTab).toHaveAttribute("aria-selected", "true", { timeout: 10_000 });
    await paneTabs.getByRole("tab", { name: "Console" }).click();
    await expect(page.locator(".computer-pane-body-computer .computer-pane-files-section")).toHaveCount(0);
    await expect(page.locator(".computer-pane-body-computer .cview-file")).toHaveCount(0);
    await filesTab.click();
    await expect(page.locator(".computer-pane-body-files .computer-pane-files-section")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator(".computer-pane-file-pill").first()).toContainText("src/App.tsx", {
      timeout: 10_000
    });
    await expect(page.locator(".cview-file").first()).toContainText("src/App.tsx", {
      timeout: 10_000
    });
    await expect(page.locator(".cview-file-body").first()).toContainText("Hello world", {
      timeout: 10_000
    });
    await expect(
      page.locator("text=Built the first draft and kept streaming the file into the workspace.")
    ).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("body")).not.toContainText("Agent Turn Loop");
    await expect(page.locator("body")).not.toContainText("Planning the app fix");
  });

  test("deep research shows a confirm card and resumes with the edited plan", async ({ page }) => {
    let createdConversationId = "";
    let userMessage = "";
    let resumed = false;
    let resumePayload: Record<string, unknown> | null = null;
    const taskId = "task-deep-plan";
    const assistantMessage = "Finished the deep research run with verified sources.";

    await page.route("**/api/apps", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ apps: [] })
      });
    });

    await page.route("**/tunnel/status", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({})
      });
    });

    await page.route("**/conversations?**", async (route) => {
      const conversations = createdConversationId
        ? [
            {
              id: createdConversationId,
              title: "Deep research preview",
              channel: "web",
              created_at: "2026-04-03T10:00:00.000Z",
              updated_at: resumed ? "2026-04-03T10:03:00.000Z" : "2026-04-03T10:01:00.000Z",
              message_count: resumed ? 2 : 1,
              archived: false
            }
          ]
        : [];
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations,
          total: conversations.length,
          limit: 30,
          offset: 0
        })
      });
    });

    await page.route("**/conversations/*/messages?**", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages:
            conversationId && conversationId === createdConversationId
              ? resumed
                ? [
                    {
                      id: "msg-user-deep-research",
                      role: "user",
                      content: userMessage,
                      timestamp: "2026-04-03T10:00:01.000Z",
                      model_used: null,
                      trace_id: null
                    },
                    {
                      id: "msg-assistant-deep-research",
                      role: "assistant",
                      content: assistantMessage,
                      timestamp: "2026-04-03T10:03:10.000Z",
                      model_used: "test-model",
                      trace_id: null
                    }
                  ]
                : [
                    {
                      id: "msg-user-deep-research",
                      role: "user",
                      content: userMessage,
                      timestamp: "2026-04-03T10:00:01.000Z",
                      model_used: null,
                      trace_id: null
                    }
                  ]
              : []
        })
      });
    });

    await page.route("**/conversations/*", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      if (!conversationId || conversationId !== createdConversationId) {
        await route.fulfill({
          status: 404,
          contentType: "application/json",
          body: JSON.stringify({ error: "not found" })
        });
        return;
      }
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          id: createdConversationId,
          title: "Deep research preview",
          channel: "web",
          created_at: "2026-04-03T10:00:00.000Z",
          updated_at: resumed ? "2026-04-03T10:03:10.000Z" : "2026-04-03T10:01:00.000Z",
          message_count: resumed ? 2 : 1
        })
      });
    });

    await page.route("**/chat/stream", async (route) => {
      const payload = route.request().postDataJSON() as {
        conversation_id?: string;
        message?: string;
        deep_research?: boolean;
        plan_confirmation_mode?: string;
      };
      expect(payload.deep_research).toBe(true);
      expect(payload.plan_confirmation_mode).toBe("before_execution");
      createdConversationId = payload.conversation_id || "conv-deep-research";
      userMessage = payload.message || "compare open source release strategies for ai agents";
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: [
          `event: task_started\ndata: {"task_id":"${taskId}","description":"Deep research preview","status":"in_progress","work_type":"research","conversation_id":"${createdConversationId}"}\n\n`,
          `event: plan_generated\ndata: {"step_type":"plan_generated","plan":{"plan_id":"plan-preview","revision":1,"summary":"","steps":[{"id":1,"title":"Scope the question","description":"Clarify the research goal and constraints.","status":"pending","action":null,"arguments":{},"tool_hint":null},{"id":2,"title":"Gather source sets","description":"Collect primary sources, recent reporting, and comparison points.","status":"pending","action":null,"arguments":{},"tool_hint":null},{"id":3,"title":"Verify and synthesize","description":"Compare claims, resolve contradictions, and answer with citations.","status":"pending","action":null,"arguments":{},"tool_hint":null}]}}\n\n`,
          `event: plan_ready_for_confirmation\ndata: {"step_type":"plan_ready_for_confirmation","task_id":"${taskId}","source":"deep_research","plan":{"plan_id":"plan-preview","revision":1,"summary":"","steps":[{"id":1,"title":"Scope the question","description":"Clarify the research goal and constraints.","status":"pending","action":null,"arguments":{},"tool_hint":null},{"id":2,"title":"Gather source sets","description":"Collect primary sources, recent reporting, and comparison points.","status":"pending","action":null,"arguments":{},"tool_hint":null},{"id":3,"title":"Verify and synthesize","description":"Compare claims, resolve contradictions, and answer with citations.","status":"pending","action":null,"arguments":{},"tool_hint":null}]}}\n\n`,
          "event: done\ndata: {}\n\n"
        ].join("")
      });
    });

    await page.route(`**/tasks/${taskId}/resume-chat/stream`, async (route) => {
      resumePayload = route.request().postDataJSON() as Record<string, unknown>;
      resumed = true;
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: [
          `event: task_started\ndata: {"task_id":"${taskId}","description":"Deep research preview","status":"in_progress","work_type":"research","conversation_id":"${createdConversationId}"}\n\n`,
          'event: plan_step_update\ndata: {"step_type":"plan_step_update","plan_id":"plan-preview","revision":1,"step_id":1,"step_title":"Scope the question","status":"running","detail":"Started step 1: Scope the question."}\n\n',
          `event: content\ndata: {"conversation_id":"${createdConversationId}","content":"${assistantMessage}"}\n\n`,
          "event: done\ndata: {}\n\n"
        ].join("")
      });
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    await page.locator("label").filter({ hasText: "Deep research" }).click();
    const input = page.locator("textarea[aria-label='Message']").first();
    await expect(input).toBeVisible({ timeout: 10_000 });

    await input.fill("compare open source release strategies for ai agents");
    await input.press("Enter");

    await expect(page.locator("text=Plan ready")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("text=Review the plan, make edits if needed, then start the run.")).toBeVisible({
      timeout: 10_000
    });

    await page.getByRole("button", { name: "Edit" }).click();
    await page.getByPlaceholder("Add a brief research summary").fill(
      "Edited summary for a source-backed open source release strategy review."
    );
    await page.getByRole("button", { name: "Start" }).click();

    await expect(page.locator(`text=${assistantMessage}`)).toBeVisible({ timeout: 10_000 });
    expect(
      (resumePayload?.plan_override as { summary?: string } | undefined)?.summary
    ).toBe("Edited summary for a source-backed open source release strategy review.");
  });

  test("deleting the active chat loads the next available conversation", async ({ page }) => {
    const currentConversationId = "conv-delete-current";
    const nextConversationId = "conv-delete-next";
    const createdAt = "2026-03-12T10:00:00.000Z";
    const updatedAt = "2026-03-12T10:05:00.000Z";
    let conversations = [
      {
        id: currentConversationId,
        title: "Current chat slated for delete",
        channel: "web",
        created_at: createdAt,
        updated_at: updatedAt,
        message_count: 2,
        archived: false
      },
      {
        id: nextConversationId,
        title: "Fallback chat after delete",
        channel: "web",
        created_at: createdAt,
        updated_at: "2026-03-12T10:04:00.000Z",
        message_count: 2,
        archived: false
      }
    ];
    const messagesByConversation: Record<string, Array<Record<string, unknown>>> = {
      [currentConversationId]: [
        {
          id: "msg-delete-user",
          role: "user",
          content: "delete the current chat when done",
          timestamp: createdAt,
          model_used: null,
          trace_id: null
        },
        {
          id: "msg-delete-assistant",
          role: "assistant",
          content: "Current conversation reply.",
          timestamp: updatedAt,
          model_used: "test-model",
          trace_id: null
        }
      ],
      [nextConversationId]: [
        {
          id: "msg-fallback-user",
          role: "user",
          content: "show the fallback chat after delete",
          timestamp: createdAt,
          model_used: null,
          trace_id: null
        },
        {
          id: "msg-fallback-assistant",
          role: "assistant",
          content: "Next conversation reply.",
          timestamp: updatedAt,
          model_used: "test-model",
          trace_id: null
        }
      ]
    };

    await page.setViewportSize({ width: 1700, height: 1100 });
    await page.addInitScript((conversationId: string) => {
      window.sessionStorage.setItem("agentark.chat.lastConversationId", conversationId);
    }, currentConversationId);

    await page.route("**/conversations?**", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations,
          total: conversations.length,
          limit: 30,
          offset: 0
        })
      });
    });

    await page.route(`**/conversations/${currentConversationId}`, async (route) => {
      if (route.request().method().toUpperCase() === "DELETE") {
        conversations = conversations.filter((conversation) => conversation.id !== currentConversationId);
        await route.fulfill({
          contentType: "application/json",
          body: JSON.stringify({ ok: true })
        });
        return;
      }
      const conversation = conversations.find((item) => item.id === currentConversationId);
      if (!conversation) {
        await route.fulfill({
          status: 404,
          contentType: "application/json",
          body: JSON.stringify({ error: "not found" })
        });
        return;
      }
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(conversation)
      });
    });

    await page.route(`**/conversations/${nextConversationId}`, async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify(conversations.find((item) => item.id === nextConversationId))
      });
    });

    await page.route("**/conversations/*/messages?**", async (route) => {
      const url = new URL(route.request().url());
      const conversationId = url.pathname.split("/")[2] || "";
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          messages: messagesByConversation[conversationId] || []
        })
      });
    });

    page.on("dialog", async (dialog) => {
      if (dialog.type() === "confirm") {
        await dialog.accept();
      }
    });

    await page.goto("/");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    const chatNav = page.locator("text=Chat").first();
    if (await chatNav.isVisible()) {
      await chatNav.click();
    }

    await expect(page.locator("text=Current conversation reply.")).toBeVisible({ timeout: 10_000 });

    const currentConversationCard = page
      .locator(".conversation-card", { hasText: "Current chat slated for delete" })
      .first();
    await expect(currentConversationCard).toBeVisible({ timeout: 10_000 });
    await currentConversationCard.locator(".conversation-card-menu").click();
    await page.getByRole("menuitem", { name: "Delete chat" }).click();

    await expect(page.locator("text=Chat deleted.")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("text=Next conversation reply.")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("text=Current conversation reply.")).not.toBeVisible();
  });
});
