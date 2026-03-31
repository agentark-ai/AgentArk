import { test, expect } from "@playwright/test";

test.describe("Project workspace UI @smoke", () => {
  test("chat workspace scope filters conversations and sends project_id", async ({ page }) => {
    await page.setViewportSize({ width: 1720, height: 1120 });
    const projects = [
      {
        id: "proj-alpha",
        name: "Alpha Project",
        description: "Scoped workspace",
        created_at: "2026-03-31T09:00:00.000Z",
        updated_at: "2026-03-31T09:00:00.000Z",
      },
    ];
    let createdConversationId = "";
    let sentProjectId = "";

    await page.route("**/projects", async (route) => {
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ projects }),
      });
    });

    await page.route("**/conversations?**", async (route) => {
      const url = new URL(route.request().url());
      const projectId = url.searchParams.get("project_id") || "";
      const conversations = projectId
        ? [
            {
              id: "conv-alpha-1",
              title: "Alpha backlog",
              channel: "web",
              project_id: "proj-alpha",
              created_at: "2026-03-31T09:10:00.000Z",
              updated_at: "2026-03-31T09:11:00.000Z",
              message_count: 1,
              archived: false,
            },
          ]
        : [
            {
              id: "conv-global-1",
              title: "Global backlog",
              channel: "web",
              project_id: null,
              created_at: "2026-03-31T09:05:00.000Z",
              updated_at: "2026-03-31T09:06:00.000Z",
              message_count: 1,
              archived: false,
            },
          ];

      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations,
          total: conversations.length,
          limit: 20,
          offset: 0,
        }),
      });
    });

    await page.route("**/chat/stream", async (route) => {
      const payload = route.request().postDataJSON() as {
        conversation_id?: string;
        message?: string;
        project_id?: string;
      };
      createdConversationId = payload.conversation_id || "conv-alpha-new";
      sentProjectId = payload.project_id || "";
      await route.fulfill({
        status: 200,
        contentType: "text/event-stream",
        body: [
          `event: content\ndata: {"conversation_id":"${createdConversationId}","content":"Scoped reply ready."}\n\n`,
          "event: done\ndata: {}\n\n",
        ].join(""),
      });
    });

    await page.goto("/ui/chat");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    await expect(page.locator("text=Global backlog")).toBeVisible({ timeout: 10_000 });

    const scopeSelect = page.getByRole("combobox", { name: "Workspace scope" });
    await scopeSelect.click();
    await page.getByRole("option", { name: "Alpha Project" }).click();

    await expect(page.locator("text=Alpha backlog")).toBeVisible({ timeout: 10_000 });
    await expect(page.locator("text=Global backlog")).toHaveCount(0);

    const input = page.locator("textarea[aria-label='Message']").first();
    await expect(input).toBeVisible({ timeout: 10_000 });
    await input.fill("scope check");
    await input.press("Enter");

    await expect.poll(() => sentProjectId).toBe("proj-alpha");
    await expect(page.locator("text=Scoped reply ready.")).toBeVisible({ timeout: 10_000 });
  });

  test("creating a project activates its workspace and returns to chat", async ({ page }) => {
    await page.setViewportSize({ width: 1720, height: 1120 });
    let projects: Array<Record<string, unknown>> = [];
    const requestedProjectIds: string[] = [];

    await page.route("**/projects", async (route) => {
      if (route.request().method() === "POST") {
        const payload = route.request().postDataJSON() as { name?: string; description?: string };
        projects = [
          {
            id: "proj-launch",
            name: payload.name || "Launch Project",
            description: payload.description || "",
            created_at: "2026-03-31T10:00:00.000Z",
            updated_at: "2026-03-31T10:00:00.000Z",
          },
        ];
        await route.fulfill({
          contentType: "application/json",
          body: JSON.stringify({ id: "proj-launch", status: "ok" }),
        });
        return;
      }

      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({ projects }),
      });
    });

    await page.route("**/conversations?**", async (route) => {
      const url = new URL(route.request().url());
      requestedProjectIds.push(url.searchParams.get("project_id") || "");
      await route.fulfill({
        contentType: "application/json",
        body: JSON.stringify({
          conversations: [],
          total: 0,
          limit: 20,
          offset: 0,
        }),
      });
    });

    await page.goto("/ui/projects");
    await page.waitForSelector("text=AgentArk", { timeout: 15_000 });

    await page.getByRole("button", { name: "New Project" }).click();
    await page.getByLabel("Name").fill("Launch Project");
    await page.getByLabel("Description").fill("Workspace activation flow");
    await page.getByRole("button", { name: "Create" }).click();

    await expect(page.locator("textarea[aria-label='Message']").first()).toBeVisible({
      timeout: 10_000,
    });
    await expect.poll(() => requestedProjectIds.includes("proj-launch")).toBeTruthy();
  });
});
