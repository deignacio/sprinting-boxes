import { BrowserRouter as Router, Routes, Route } from "react-router-dom";
import ListPage from "./pages/ListPage";
import CreateRunPage from "./pages/CreateRunPage";
import RunDetailsPage from "./pages/RunDetailsPage";

function App() {
  return (
    <Router>
      <Routes>
        <Route path="/" element={<ListPage />} />
        <Route path="/create-run" element={<CreateRunPage />} />
        <Route path="/run-details/:id" element={<RunDetailsPage />} />
      </Routes>
    </Router>
  );
}

export default App;
