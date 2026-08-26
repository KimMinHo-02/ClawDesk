import { AutomationsFeature } from "./features/automations/AutomationsFeature";
import { ChannelsFeature } from "./features/channels/ChannelsFeature";
import { ModelsFeature } from "./features/models/ModelsFeature";
import { PluginsFeature } from "./features/plugins/PluginsFeature";
import { ProfileFeature } from "./features/profile/ProfileFeature";
import { SetupFeature } from "./features/setup/SetupFeature";
import { SkillsFeature } from "./features/skills/SkillsFeature";
import { ToolsSecurityFeature } from "./features/tools-security/ToolsSecurityFeature";
import { getStrings } from "./i18n/ko";

const app = getStrings("app");

function App() {
  return (
    <main>
      <h1>{app.title}</h1>
      <p>{app.subtitle}</p>
      <SetupFeature />
      <ModelsFeature />
      <SkillsFeature />
      <PluginsFeature />
      <ToolsSecurityFeature />
      <ChannelsFeature />
      <AutomationsFeature />
      <ProfileFeature />
    </main>
  );
}

export default App;
