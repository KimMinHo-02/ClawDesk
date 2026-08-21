import { ModelsFeature } from "./features/models/ModelsFeature";
import { SetupFeature } from "./features/setup/SetupFeature";
import { getStrings } from "./i18n/ko";

const app = getStrings("app");

function App() {
  return (
    <main>
      <h1>{app.title}</h1>
      <p>{app.subtitle}</p>
      <SetupFeature />
      <ModelsFeature />
    </main>
  );
}

export default App;
