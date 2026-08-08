import { expect, test } from "@playwright/test";

import { installMockBridge } from "../helpers/bridge";

test.beforeEach(async ({ page }) => {
  await installMockBridge(page);
});

test("channel header opens and closes the coding workspace", async ({
  page,
}) => {
  await page.goto("/");
  await page.getByTestId("channel-engineering").click();
  await expect(page.getByTestId("chat-title")).toHaveText("engineering");

  const workspace = page.getByTestId("coding-workspace");
  const workspaceSurface = workspace.locator("..");

  await expect(workspaceSurface).toHaveAttribute("inert", "");
  await expect(workspaceSurface).toHaveClass(/opacity-0/);

  await page.getByTestId("open-coding-workspace").click();

  await expect(workspaceSurface).not.toHaveAttribute("inert", "");
  await expect(workspaceSurface).toHaveClass(/opacity-100/);
  await expect(
    page.getByRole("heading", { name: "Start from a project" }),
  ).toBeVisible();

  await page.getByTestId("close-coding-workspace").click();

  await expect(workspaceSurface).toHaveAttribute("inert", "");
  await expect(workspaceSurface).toHaveClass(/opacity-0/);
});
