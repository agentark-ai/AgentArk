import type { SkillImportResponse } from "../../types";
import { asRecord, num, str, toBool, type JsonRecord } from "./pageHelpers";
import {
  dedupeStrings,
  extractFirstUrl,
  type HookTriggerValue,
  inferHookTriggerFromInstruction,
  inferTaskCronFromInstruction,
  isHookAttachedToAction,
  isHookRecordAttachedToAction,
  sanitizeHookName,
} from "./workspaceCore";

const IMPORT_RISK_POLICY = {
  forceThreshold: 8,
  maxScore: 10,
  contextualStrongRatio: 0.8,
  contextualStrongScoreCap: 4,
  contextualPartialRatio: 0.5,
  contextualPartialScoreCap: 6,
  suspiciousContextualRatio: 0.5,
  reviewFloor: 5,
  riskyFloor: 8.5,
  secureBandMax: 5,
  reviewBandMax: 8,
  contextualCredentialSeverityMax: 2,
  contextualHomePathSeverityMax: 4,
  findingMediumSeverity: 3,
  findingHighSeverity: 6,
} as const;

export const IMPORT_SECURITY_FORCE_RISK_THRESHOLD =
  IMPORT_RISK_POLICY.forceThreshold;
export { IMPORT_RISK_POLICY };

export type ImportRiskBand = "secure" | "review" | "risky";

export type SkillImportSummary = {
  result: SkillImportResponse;
  message?: string;
};

export type SkillEditorForm = {
  name: string;
  description: string;
  version: string;
  requiredInputsCsv: string;
  emoji: string;
  toolsCsv: string;
  workflow: string;
};

function isContextualCredentialFinding(finding: JsonRecord): boolean {
  if ("contextual" in finding) return toBool(finding.contextual);
  const matchedText = str(finding.matched_text, "");
  return (
    num(finding.severity, 0) <=
      IMPORT_RISK_POLICY.contextualCredentialSeverityMax ||
    matchedText.includes("$") ||
    matchedText.includes("${")
  );
}

function normalizedImportMatchText(finding: JsonRecord): string {
  return str(finding.matched_text, "").trim().replace(/\\/g, "/").toLowerCase();
}

function isLowRiskHomePathFinding(finding: JsonRecord): boolean {
  if ("contextual" in finding) return toBool(finding.contextual);
  const normalized = normalizedImportMatchText(finding);
  return (
    num(finding.severity, 0) <=
      IMPORT_RISK_POLICY.contextualHomePathSeverityMax &&
    (normalized === "~" || normalized === "~/" || normalized.startsWith("~/"))
  );
}

function isContextualImportFinding(finding: JsonRecord): boolean {
  if ("contextual" in finding) return toBool(finding.contextual);
  const category = str(finding.category, "").toLowerCase();
  if (category === "networkaccess" || category === "environmentaccess") {
    return true;
  }
  if (category === "credentialpattern") {
    return isContextualCredentialFinding(finding);
  }
  if (category === "filesystemescape") {
    return isLowRiskHomePathFinding(finding);
  }
  return false;
}

type ImportFindingPresentation = {
  label: string;
  explanation: string | ((finding: JsonRecord) => string);
};

const IMPORT_FINDING_PRESENTATION: Record<string, ImportFindingPresentation> = {
  NetworkAccess: {
    label: "Network access",
    explanation:
      "The skill may contact the network. This is common for integrations, but review the destination before importing.",
  },
  CredentialPattern: {
    label: "Credential pattern",
    explanation: (finding) =>
      isContextualCredentialFinding(finding)
        ? "Looks like a credential example or environment variable reference. Configure the real secret in AgentArk instead of hard-coding it."
        : "Looks like a hard-coded secret or token. Do not import until the source is reviewed.",
  },
  EnvironmentAccess: {
    label: "Environment variable",
    explanation:
      "Reads environment variables. This is common for API keys, but the skill should only read the variables it needs.",
  },
  FileSystem: {
    label: "File access",
    explanation:
      "References files on the host. Review the line before importing.",
  },
  FileSystemEscape: {
    label: "Path outside workspace",
    explanation: (finding) => {
      const normalized = normalizedImportMatchText(finding);
      if (
        normalized === "~" ||
        normalized === "~/" ||
        normalized.startsWith("~/")
      ) {
        return "References your home folder. This is a review signal by itself, and becomes dangerous if the skill can run commands or read/write files there.";
      }
      if (normalized.includes("../..")) {
        return "Uses parent-directory traversal, which can reach files outside the skill workspace.";
      }
      return "References a host or system path outside the skill workspace. Override only if you trust the source and this access is expected.";
    },
  },
  ShellExecution: {
    label: "Command execution",
    explanation:
      "The skill may run commands. Import only from a source you trust.",
  },
  CodeExecution: {
    label: "Code execution",
    explanation: "The skill may run code. Import only from a source you trust.",
  },
  EncodedPayload: {
    label: "Encoded payload",
    explanation:
      "Contains encoded or obfuscated content. Review carefully because it can hide behavior.",
  },
  SupplyChain: {
    label: "Package install",
    explanation:
      "Installs or fetches dependencies. Review the package source before importing.",
  },
  DataExfiltration: {
    label: "Data exfiltration",
    explanation:
      "May move data out of AgentArk. Review carefully before importing.",
  },
};

export function importFindingCategoryLabel(
  category: string,
  finding?: JsonRecord,
): string {
  const serverLabel = finding ? str(finding.label, "").trim() : "";
  if (serverLabel) return serverLabel;
  return (
    IMPORT_FINDING_PRESENTATION[category]?.label ||
    category
      .replace(/([A-Z])/g, " $1")
      .trim()
      .toLowerCase()
  );
}

export function explainImportFinding(finding: JsonRecord): string {
  const serverExplanation = str(finding.explanation, "").trim();
  if (serverExplanation) return serverExplanation;
  const category = str(finding.category, "");
  const explanation = IMPORT_FINDING_PRESENTATION[category]?.explanation;
  if (typeof explanation === "function") return explanation(finding);
  return (
    explanation ||
    str(finding.description, "") ||
    "Review this signal before importing."
  );
}

export function computeImportRiskSummary(
  security: SkillImportResponse["security"] | null | undefined,
): {
  score10: number;
  band: ImportRiskBand;
  bandLabel: string;
  chipColor: "success" | "warning" | "error";
  rawSeverity: number;
  totalFindings: number;
  contextualFindings: number;
} {
  if (!security) {
    return {
      score10: 0,
      band: "secure",
      bandLabel: "Secure",
      chipColor: "success",
      rawSeverity: 0,
      totalFindings: 0,
      contextualFindings: 0,
    };
  }

  const findings = Array.isArray(security.findings) ? security.findings : [];
  const findingRecords = findings.map((item) => asRecord(item));
  const explicitSeverity = Math.max(0, num(security.total_severity, 0));
  const summedSeverity = findingRecords.reduce(
    (sum, finding) => sum + Math.max(0, num(finding.severity, 0)),
    0,
  );
  const rawSeverity = explicitSeverity > 0 ? explicitSeverity : summedSeverity;
  const serverTotalFindings = num(security.total_findings, -1);
  const serverContextualFindings = num(security.contextual_findings, -1);
  const totalFindings =
    serverTotalFindings >= 0 ? serverTotalFindings : findingRecords.length;
  const contextualFindings =
    serverContextualFindings >= 0
      ? Math.min(totalFindings, serverContextualFindings)
      : findingRecords.filter((finding) => isContextualImportFinding(finding))
          .length;
  const contextualRatio =
    totalFindings > 0 ? contextualFindings / totalFindings : 0;

  const providedRiskScore = num(security.risk_score_10, -1);
  const providedBand = str(security.risk_band, "").toLowerCase();

  let score =
    providedRiskScore >= 0
      ? Math.min(IMPORT_RISK_POLICY.maxScore, providedRiskScore)
      : Math.min(IMPORT_RISK_POLICY.maxScore, rawSeverity / 4);

  if (contextualRatio >= IMPORT_RISK_POLICY.contextualStrongRatio) {
    score = Math.min(score, IMPORT_RISK_POLICY.contextualStrongScoreCap);
  } else if (contextualRatio >= IMPORT_RISK_POLICY.contextualPartialRatio) {
    score = Math.min(score, IMPORT_RISK_POLICY.contextualPartialScoreCap);
  }

  const threatLevel = str(security.threat_level, "").toLowerCase();
  if (
    threatLevel === "malicious" &&
    contextualRatio < IMPORT_RISK_POLICY.contextualStrongRatio
  ) {
    score = Math.max(score, IMPORT_RISK_POLICY.riskyFloor);
  } else if (
    threatLevel === "suspicious" &&
    contextualRatio < IMPORT_RISK_POLICY.suspiciousContextualRatio
  ) {
    score = Math.max(score, IMPORT_RISK_POLICY.reviewFloor);
  }
  if (
    toBool(security.blocked) &&
    contextualRatio < IMPORT_RISK_POLICY.contextualStrongRatio
  ) {
    score = Math.max(score, IMPORT_RISK_POLICY.riskyFloor);
  }

  const score10 = Math.max(
    0,
    Math.min(IMPORT_RISK_POLICY.maxScore, Math.round(score * 10) / 10),
  );
  const resolvedBand =
    providedBand === "secure" ||
    providedBand === "review" ||
    providedBand === "risky"
      ? (providedBand as ImportRiskBand)
      : score10 < IMPORT_RISK_POLICY.secureBandMax
        ? "secure"
        : score10 < IMPORT_RISK_POLICY.reviewBandMax
          ? "review"
          : "risky";
  if (resolvedBand === "secure") {
    return {
      score10,
      band: "secure",
      bandLabel: "Secure",
      chipColor: "success",
      rawSeverity,
      totalFindings,
      contextualFindings,
    };
  }
  if (resolvedBand === "review") {
    return {
      score10,
      band: "review",
      bandLabel: "Needs review",
      chipColor: "warning",
      rawSeverity,
      totalFindings,
      contextualFindings,
    };
  }
  return {
    score10,
    band: "risky",
    bandLabel: "Risky",
    chipColor: "error",
    rawSeverity,
    totalFindings,
    contextualFindings,
  };
}

export function defaultSkillEditorForm(name = ""): SkillEditorForm {
  return {
    name: name || "new-action",
    description: "",
    version: "1.0.0",
    requiredInputsCsv: "",
    emoji: "",
    toolsCsv: "",
    workflow: "",
  };
}

function splitActionFrontmatter(content: string): {
  frontmatter: string | null;
  body: string;
} {
  const match = content.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/);
  if (!match) return { frontmatter: null, body: content };
  return { frontmatter: match[1] ?? "", body: match[2] ?? "" };
}

function unquoteYamlScalar(value: string): string {
  const normalized = value.trim();
  if (!normalized) return "";
  if (normalized.startsWith('"') && normalized.endsWith('"')) {
    try {
      const parsed = JSON.parse(normalized);
      return typeof parsed === "string" ? parsed : normalized.slice(1, -1);
    } catch {
      return normalized.slice(1, -1);
    }
  }
  if (normalized.startsWith("'") && normalized.endsWith("'")) {
    return normalized.slice(1, -1).replace(/''/g, "'");
  }
  return normalized;
}

function quoteYamlScalar(value: string): string {
  return JSON.stringify(value ?? "");
}

function parseInlineStringArray(value: string): string[] {
  const normalized = value.trim();
  if (!normalized) return [];
  if (normalized.startsWith("[") && normalized.endsWith("]")) {
    try {
      const parsed = JSON.parse(normalized);
      if (Array.isArray(parsed)) {
        return parsed
          .map((item) => (typeof item === "string" ? item.trim() : ""))
          .filter(Boolean);
      }
    } catch {
      // Fall through to tolerant splitting below.
    }
    const raw = normalized.slice(1, -1);
    return raw
      .split(",")
      .map((item) => unquoteYamlScalar(item))
      .map((item) => item.trim())
      .filter(Boolean);
  }
  return normalized
    .split(",")
    .map((item) => unquoteYamlScalar(item))
    .map((item) => item.trim())
    .filter(Boolean);
}

function parseToolsCsv(csv: string): string[] {
  return dedupeStrings(
    csv
      .split(",")
      .map((item) => item.trim())
      .filter(Boolean),
  );
}

function parseRequiredInputsCsv(csv: string): string[] {
  return dedupeStrings(
    csv
      .split(",")
      .map((item) => item.trim().replace(/[^A-Za-z0-9_-]/g, ""))
      .filter(Boolean),
  );
}

export function parseSkillEditorForm(
  content: string,
  fallbackName: string,
): SkillEditorForm {
  const { frontmatter, body } = splitActionFrontmatter(content);
  const form = defaultSkillEditorForm(fallbackName);
  if (!frontmatter) {
    form.workflow = content.trim();
    return form;
  }

  const tools: string[] = [];
  const requiredInputs: string[] = [];
  let section: string | null = null;
  let listTarget: "tools" | "requiredInputs" | null = null;
  const lines = frontmatter.split(/\r?\n/);
  for (const rawLine of lines) {
    const line = rawLine.replace(/\t/g, "  ");
    if (!line.trim()) continue;

    const top = line.match(/^([A-Za-z0-9_-]+):\s*(.*)$/);
    if (top) {
      const key = top[1];
      const value = top[2].trim();
      section = null;
      listTarget = null;

      if (key === "name") {
        if (value) form.name = unquoteYamlScalar(value);
        continue;
      }
      if (key === "description") {
        if (value) form.description = unquoteYamlScalar(value);
        continue;
      }
      if (key === "version") {
        if (value) form.version = unquoteYamlScalar(value);
        continue;
      }
      if (
        key === "required_inputs" ||
        key === "requiredInputs" ||
        key === "required"
      ) {
        if (value) {
          requiredInputs.push(...parseInlineStringArray(value));
        } else {
          section = "required_inputs";
          listTarget = "requiredInputs";
        }
        continue;
      }
      if (key === "metadata") {
        if (value) {
          const match = value.match(/emoji\s*:\s*(.+)$/);
          if (match) form.emoji = unquoteYamlScalar(match[1]);
        } else {
          section = "metadata";
        }
        continue;
      }
      if (key === "requires") {
        if (value) {
          const match = value.match(/tools\s*:\s*(.+)$/);
          if (match) tools.push(...parseInlineStringArray(match[1]));
        } else {
          section = "requires";
        }
        continue;
      }
      continue;
    }

    const nested = line.match(/^\s{2,}([A-Za-z0-9_-]+):\s*(.*)$/);
    if (nested && section) {
      const key = nested[1];
      const value = nested[2].trim();
      listTarget = null;
      if (section === "metadata" && key === "emoji") {
        form.emoji = unquoteYamlScalar(value);
        continue;
      }
      if (section === "requires" && key === "tools") {
        if (value) {
          tools.push(...parseInlineStringArray(value));
        } else {
          listTarget = "tools";
        }
        continue;
      }
      continue;
    }

    const listItem = line.match(/^\s*-\s*(.+)$/);
    if (listItem && section === "requires" && listTarget === "tools") {
      tools.push(unquoteYamlScalar(listItem[1]));
      continue;
    }
    if (
      listItem &&
      section === "required_inputs" &&
      listTarget === "requiredInputs"
    ) {
      requiredInputs.push(unquoteYamlScalar(listItem[1]));
      continue;
    }
  }

  form.toolsCsv = dedupeStrings(tools).join(", ");
  form.requiredInputsCsv = parseRequiredInputsCsv(
    requiredInputs.join(", "),
  ).join(", ");
  form.workflow = body.trim();
  if (!form.name.trim()) form.name = fallbackName || "new-action";
  if (!form.version.trim()) form.version = "1.0.0";
  return form;
}

function extractUnknownFrontmatterLines(frontmatter: string): string[] {
  const lines = frontmatter.split(/\r?\n/);
  const kept: string[] = [];
  let skipKnownBlock = false;

  for (const line of lines) {
    const top = line.match(/^([A-Za-z0-9_-]+):\s*(.*)$/);
    if (top) {
      const key = top[1];
      if (
        key === "name" ||
        key === "description" ||
        key === "version" ||
        key === "required_inputs" ||
        key === "requiredInputs" ||
        key === "required"
      ) {
        skipKnownBlock = false;
        continue;
      }
      if (key === "metadata" || key === "requires") {
        skipKnownBlock = true;
        continue;
      }
      skipKnownBlock = false;
      kept.push(line);
      continue;
    }

    if (skipKnownBlock) {
      if (line.trim() === "" || /^\s+/.test(line)) continue;
    }
    kept.push(line);
  }

  while (kept.length > 0 && !kept[0].trim()) kept.shift();
  while (kept.length > 0 && !kept[kept.length - 1].trim()) kept.pop();
  return kept;
}

export function buildSkillMdFromForm(
  currentContent: string,
  form: SkillEditorForm,
): string {
  const { frontmatter } = splitActionFrontmatter(currentContent);
  const unknownLines = frontmatter
    ? extractUnknownFrontmatterLines(frontmatter)
    : [];
  const tools = parseToolsCsv(form.toolsCsv);
  const requiredInputs = parseRequiredInputsCsv(form.requiredInputsCsv);
  const frontmatterLines = [
    `name: ${quoteYamlScalar((form.name || "").trim())}`,
    `description: ${quoteYamlScalar((form.description || "").trim())}`,
    `version: ${quoteYamlScalar((form.version || "").trim() || "1.0.0")}`,
    `required_inputs: [${requiredInputs.map((item) => quoteYamlScalar(item)).join(", ")}]`,
    "metadata:",
    `  emoji: ${quoteYamlScalar((form.emoji || "").trim())}`,
    "requires:",
    `  tools: [${tools.map((tool) => quoteYamlScalar(tool)).join(", ")}]`,
  ];

  if (unknownLines.length > 0) {
    frontmatterLines.push("");
    frontmatterLines.push(...unknownLines);
  }

  const workflow = (form.workflow || "").trim();
  return `---\n${frontmatterLines.join("\n")}\n---\n\n${workflow}\n`;
}

export function normalizeActionName(value: string): string {
  return (value || "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9-_\s]/g, "")
    .replace(/[_\s]+/g, "-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

export function isValidActionName(value: string): boolean {
  return /^[a-z0-9-]+$/.test((value || "").trim());
}

export function isRunOnceInstruction(text: string): boolean {
  const normalized = (text || "").toLowerCase();
  return (
    normalized.includes("once") ||
    normalized.includes("now") ||
    normalized.includes("immediately")
  );
}

export function extractActionMdFromModelOutput(text: string): string {
  const raw = (text || "").trim();
  if (!raw) return "";
  const fenceRegex = /```(?:md|markdown|txt|yaml)?\s*([\s\S]*?)```/gi;
  const blocks: string[] = [];
  let match: RegExpExecArray | null = null;
  while ((match = fenceRegex.exec(raw)) !== null) {
    blocks.push((match[1] || "").trim());
  }
  for (const block of blocks) {
    if (block.startsWith("---")) return block;
  }
  if (blocks.length > 0) return blocks[0];
  if (raw.startsWith("---")) return raw;
  const frontmatterIdx = raw.indexOf("\n---\n");
  if (frontmatterIdx >= 0) {
    const maybe = raw.slice(raw.lastIndexOf("---", frontmatterIdx - 1)).trim();
    if (maybe.startsWith("---")) return maybe;
  }
  return raw;
}

export {
  dedupeStrings,
  extractFirstUrl,
  inferHookTriggerFromInstruction,
  inferTaskCronFromInstruction,
  isHookAttachedToAction,
  isHookRecordAttachedToAction,
  sanitizeHookName,
  type HookTriggerValue,
};
