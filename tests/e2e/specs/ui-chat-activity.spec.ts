import { test, expect } from "@playwright/test";

test.describe("Chat Activity UI @smoke", () => {
  test("sending first message in a draft chat does not blank the UI", async ({ page }) => {
    let createdConversationId = "";
    let userMessage = "";
    const assistantMessage = "Saved your preference and kept the chat UI stable.";
    const pageErrors: string[] = [];

    page.on("pageerror", (error) => {
      pageErrors.push(String(error));
    });

    await page.route("**/projects", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ projects: [] })
      });
    });

    await page.route("**/conversations?**", async (route) => {
      const conversations = createdConversationId
        ? [
            {
              id: createdConversationId,
              title: "Preference capture chat",
              channel: "web",
              project_id: null,
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
          project_id: null,
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
              project_id: null,
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
          project_id: null,
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
});
