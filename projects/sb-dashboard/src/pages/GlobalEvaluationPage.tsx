import React from "react";
import { useNavigate } from "react-router-dom";
import GlobalEvaluation from "../components/GlobalEvaluation/GlobalEvaluation";

const GlobalEvaluationPage: React.FC = () => {
  const navigate = useNavigate();

  return (
    <div className="container mx-auto px-8 py-8">
      <div style={{ marginBottom: "20px" }}>
        <button
          onClick={() => navigate("/")}
          style={{
            padding: "8px 16px",
            background: "#334155",
            color: "#e2e8f0",
            border: "none",
            borderRadius: "6px",
            cursor: "pointer",
            fontSize: "0.9rem",
          }}
        >
          ← Back to Runs
        </button>
      </div>

      <GlobalEvaluation />
    </div>
  );
};

export default GlobalEvaluationPage;
