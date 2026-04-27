import SettingsPageFull from "./SettingsPageFull";

type ArkPulsePageProps = {
  autoRefresh: boolean;
};

export default function ArkPulsePage({
  autoRefresh,
}: ArkPulsePageProps) {
  return (
    <SettingsPageFull
      autoRefresh={autoRefresh}
      initialTab={9}
      hideSettingsNav
      standaloneSurface="arkpulse"
    />
  );
}
