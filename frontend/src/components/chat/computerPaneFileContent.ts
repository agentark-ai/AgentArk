export interface ComputerPaneFileContentSources {
  workspaceContent?: string;
  fallbackContent?: string;
  liveWriteContent?: string;
  isLiveWrite?: boolean;
  liveWriteActive?: boolean;
}

export function isOmittedContentPlaceholder(value: string): boolean {
  return /^\[omitted\s+[\d,]+\s+chars?(?:\s*\/\s*[\d,]+\s+lines?)?\]$/i.test(
    (value || "").trim(),
  );
}

function usableFileContent(value = ""): string {
  return isOmittedContentPlaceholder(value) ? "" : value;
}

export function resolveComputerPaneFileContent({
  workspaceContent = "",
  fallbackContent = "",
  liveWriteContent = "",
  isLiveWrite = false,
  liveWriteActive = false,
}: ComputerPaneFileContentSources): string {
  const liveContent = usableFileContent(liveWriteContent);
  const workspace = usableFileContent(workspaceContent);
  const fallback = usableFileContent(fallbackContent);

  if (isLiveWrite && liveWriteActive && liveContent) {
    return liveContent;
  }

  return workspace || fallback || (isLiveWrite ? liveContent : "");
}
