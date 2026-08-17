import { getStrings } from "./i18n/ko";

const app = getStrings("app");

function App() {
  return (
    <main>
      <h1>{app.title}</h1>
      <p>{app.subtitle}</p>
    </main>
  );
}

export default App;
