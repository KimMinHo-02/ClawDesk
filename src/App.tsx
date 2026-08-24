import { ModelsFeature } from "./features/models/ModelsFeature";
import { PluginsFeature } from "./features/plugins/PluginsFeature";
import { SetupFeature } from "./features/setup/SetupFeature";
import { SkillsFeature } from "./features/skills/SkillsFeature";
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
    </main>
  );
}

export default App;
