import React, { useState } from "react";
import "./GlobalEvaluation.css";

interface DetectorConfig {
  min_drop: number;
  min_prepoint_duration: number;
  min_post_duration: number;
  max_post_proba: number;
  absolute_threshold: number;
  min_gap: number;
  smoothing_window: number;
  field_onset: number;
}

interface AggregatedMetrics {
  config: DetectorConfig;
  total_tp: number;
  total_fp: number;
  total_fn: number;
  total_runs_with_data: number;
  precision: number;
  recall: number;
  f1: number;
  fn_causes: Record<string, number>;
  fp_causes: Record<string, number>;
}

interface GlobalSweepResponse {
  results: AggregatedMetrics[];
  baseline: AggregatedMetrics;
  runs_evaluated: number;
}

const GlobalEvaluation: React.FC = () => {
  const [config, setConfig] = useState<DetectorConfig | null>(null);
  const [sweepRanges, setSweepRanges] = useState({
    min_prepoint_duration: "",
    min_post_duration: "",
    max_post_proba: "",
    absolute_threshold: "",
    min_drop: "",
    min_gap: "",
    smoothing_window: "",
    field_onset: "",
  });

  const [sweepResults, setSweepResults] = useState<AggregatedMetrics[]>([]);
  const [sweepBaseline, setSweepBaseline] = useState<AggregatedMetrics | null>(null);
  const [runsEvaluated, setRunsEvaluated] = useState(0);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [expandedRow, setExpandedRow] = useState<number | null>(null);

  // Load detector config on mount and set suggested sweep ranges
  React.useEffect(() => {
    const loadConfig = async () => {
      try {
        const response = await fetch("/api/evaluation/config");
        if (response.ok) {
          const cfg = (await response.json()) as DetectorConfig;
          setConfig(cfg);
          // Set suggested ranges relative to the loaded config
          setSweepRanges({
            min_prepoint_duration: `${cfg.min_prepoint_duration}`,
            min_post_duration: `${cfg.min_post_duration}`,
            max_post_proba: `${cfg.max_post_proba}`,
            absolute_threshold: `${cfg.absolute_threshold}`,
            min_drop: `${cfg.min_drop}`,
            min_gap: `${cfg.min_gap}`,
            smoothing_window: `${cfg.smoothing_window}`,
            field_onset: `${cfg.field_onset}`,
          });
        }
      } catch (err) {
        console.error("Failed to load detector config", err);
      }
    };
    loadConfig();
  }, []);

  const runGlobalSweep = async () => {
    setLoading(true);
    setError(null);

    try {
      // Parse sweep ranges from comma-separated strings
      const ranges: Record<string, number[]> = {};

      if (sweepRanges.min_prepoint_duration) {
        ranges.min_prepoint_duration = sweepRanges.min_prepoint_duration
          .split(",")
          .map((s) => parseInt(s.trim()));
      }
      if (sweepRanges.min_post_duration) {
        ranges.min_post_duration = sweepRanges.min_post_duration
          .split(",")
          .map((s) => parseInt(s.trim()));
      }
      if (sweepRanges.max_post_proba) {
        ranges.max_post_proba = sweepRanges.max_post_proba.split(",").map((s) => parseFloat(s.trim()));
      }
      if (sweepRanges.absolute_threshold) {
        ranges.absolute_threshold = sweepRanges.absolute_threshold
          .split(",")
          .map((s) => parseFloat(s.trim()));
      }
      if (sweepRanges.min_drop) {
        ranges.min_drop = sweepRanges.min_drop.split(",").map((s) => parseFloat(s.trim()));
      }
      if (sweepRanges.min_gap) {
        ranges.min_gap = sweepRanges.min_gap.split(",").map((s) => parseInt(s.trim()));
      }
      if (sweepRanges.smoothing_window) {
        ranges.smoothing_window = sweepRanges.smoothing_window
          .split(",")
          .map((s) => parseInt(s.trim()));
      }
      if (sweepRanges.field_onset) {
        ranges.field_onset = sweepRanges.field_onset.split(",").map((s) => parseFloat(s.trim()));
      }

      const sweepRequest = {
        ranges: Object.fromEntries(
          Object.entries(ranges).map(([key, values]) => [key, values.map((v) => v)]),
        ),
      };

      const response = await fetch(`/api/evaluation/sweep-all`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(sweepRequest),
      });

      if (!response.ok) {
        throw new Error(`HTTP ${response.status}`);
      }

      const data = (await response.json()) as GlobalSweepResponse;
      setSweepResults(data.results);
      setSweepBaseline(data.baseline);
      setRunsEvaluated(data.runs_evaluated);
    } catch (err) {
      setError(`Sweep failed: ${err instanceof Error ? err.message : String(err)}`);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="global-evaluation">
      <h2>Global Cliff Detector Tuning (All Runs)</h2>

      <div className="sweep-section">
        <h3>Parameter Ranges to Sweep</h3>
        <p className="sweep-instruction">Enter comma-separated values for any parameter. Leave empty to use default value.</p>

        <div className="sweep-form">
          <div className="param-group">
            <label>
              min_prepoint_duration
              <span className="param-default">(default: {config?.min_prepoint_duration ?? 10})</span>
              <input
                type="text"
                value={sweepRanges.min_prepoint_duration}
                onChange={(e) =>
                  setSweepRanges({ ...sweepRanges, min_prepoint_duration: e.target.value })
                }
                placeholder="e.g., 10,15,20"
              />
            </label>
          </div>

          <div className="param-group">
            <label>
              min_post_duration
              <span className="param-default">(default: {config?.min_post_duration ?? 10})</span>
              <input
                type="text"
                value={sweepRanges.min_post_duration}
                onChange={(e) =>
                  setSweepRanges({ ...sweepRanges, min_post_duration: e.target.value })
                }
                placeholder="e.g., 10,15,20"
              />
            </label>
          </div>

          <div className="param-group">
            <label>
              max_post_proba
              <span className="param-default">(default: {config?.max_post_proba?.toFixed(2) ?? "0.55"})</span>
              <input
                type="text"
                value={sweepRanges.max_post_proba}
                onChange={(e) => setSweepRanges({ ...sweepRanges, max_post_proba: e.target.value })}
                placeholder="e.g., 0.55,0.45,0.40"
              />
            </label>
          </div>

          <div className="param-group">
            <label>
              absolute_threshold
              <span className="param-default">(default: {config?.absolute_threshold?.toFixed(2) ?? "0.50"})</span>
              <input
                type="text"
                value={sweepRanges.absolute_threshold}
                onChange={(e) =>
                  setSweepRanges({ ...sweepRanges, absolute_threshold: e.target.value })
                }
                placeholder="e.g., 0.50,0.40,0.35"
              />
            </label>
          </div>

          <div className="param-group">
            <label>
              min_drop
              <span className="param-default">(default: {config?.min_drop?.toFixed(2) ?? "0.15"})</span>
              <input
                type="text"
                value={sweepRanges.min_drop}
                onChange={(e) => setSweepRanges({ ...sweepRanges, min_drop: e.target.value })}
                placeholder="e.g., 0.15,0.20,0.25"
              />
            </label>
          </div>

          <div className="param-group">
            <label>
              min_gap
              <span className="param-default">(default: {config?.min_gap ?? 20})</span>
              <input
                type="text"
                value={sweepRanges.min_gap}
                onChange={(e) => setSweepRanges({ ...sweepRanges, min_gap: e.target.value })}
                placeholder="e.g., 20,40,60"
              />
            </label>
          </div>

          <div className="param-group">
            <label>
              smoothing_window
              <span className="param-default">(default: {config?.smoothing_window ?? 3})</span>
              <input
                type="text"
                value={sweepRanges.smoothing_window}
                onChange={(e) =>
                  setSweepRanges({ ...sweepRanges, smoothing_window: e.target.value })
                }
                placeholder="e.g., 3,5,7"
              />
            </label>
          </div>

          <div className="param-group">
            <label>
              field_onset
              <span className="param-default">(default: {config?.field_onset?.toFixed(2) ?? "1.5"})</span>
              <input
                type="text"
                value={sweepRanges.field_onset}
                onChange={(e) =>
                  setSweepRanges({ ...sweepRanges, field_onset: e.target.value })
                }
                placeholder="e.g., 1.5,1.2,1.0,0.8"
              />
            </label>
          </div>

          <button className="sweep-btn" onClick={runGlobalSweep} disabled={loading}>
            {loading ? "Running sweep..." : "Run Global Sweep"}
          </button>
        </div>

        {error && <div className="error-message">{error}</div>}
      </div>

      {sweepBaseline && (
        <div className="results-section">
          <div className="baseline-box">
            <h3>Baseline Configuration (Default Parameters)</h3>
            <div className="metrics-grid">
              <div className="metric">
                <span className="metric-label">TP</span>
                <span className="metric-value">{sweepBaseline.total_tp}</span>
              </div>
              <div className="metric">
                <span className="metric-label">FP</span>
                <span className="metric-value">{sweepBaseline.total_fp}</span>
              </div>
              <div className="metric">
                <span className="metric-label">FN</span>
                <span className="metric-value">{sweepBaseline.total_fn}</span>
              </div>
              <div className="metric">
                <span className="metric-label">Precision</span>
                <span className="metric-value">{(sweepBaseline.precision * 100).toFixed(1)}%</span>
              </div>
              <div className="metric">
                <span className="metric-label">Recall</span>
                <span className="metric-value">{(sweepBaseline.recall * 100).toFixed(1)}%</span>
              </div>
              <div className="metric">
                <span className="metric-label">F1</span>
                <span className="metric-value">{sweepBaseline.f1.toFixed(3)}</span>
              </div>
            </div>
            <p className="runs-count">Evaluated across {runsEvaluated} runs</p>

            {(sweepBaseline.total_fn > 0 || sweepBaseline.total_fp > 0) && (
              <div className="causes-container">
                {sweepBaseline.total_fn > 0 && Object.values(sweepBaseline.fn_causes).some(c => c > 0) && (
                  <div className="fn-causes-section">
                    <h4>False Negative Root Causes</h4>
                    <div className="fn-causes-list">
                      {Object.entries(sweepBaseline.fn_causes)
                        .filter(([_, count]) => count > 0)
                        .map(([cause, count]) => (
                          <div key={cause} className="fn-cause-item">
                            <span className="cause-name">{cause.replace(/_/g, " ")}</span>
                            <span className="cause-count">{count}</span>
                          </div>
                        ))}
                    </div>
                  </div>
                )}
                {sweepBaseline.total_fp > 0 && Object.values(sweepBaseline.fp_causes).some(c => c > 0) && (
                  <div className="fp-causes-section">
                    <h4>False Positive Root Causes</h4>
                    <div className="fp-causes-list">
                      {Object.entries(sweepBaseline.fp_causes)
                        .filter(([_, count]) => count > 0)
                        .map(([cause, count]) => (
                          <div key={cause} className="fp-cause-item">
                            <span className="cause-name">{cause.replace(/_/g, " ")}</span>
                            <span className="cause-count">{count}</span>
                          </div>
                        ))}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>

          {sweepResults.length > 0 && (
            <div className="results-table-section">
              <h3>Top Configurations ({sweepResults.length} tested)</h3>
              <div className="table-container">
                <table className="results-table">
                  <thead>
                    <tr>
                      <th>Config</th>
                      <th>TP</th>
                      <th>FP</th>
                      <th>FN</th>
                      <th>Precision</th>
                      <th>Recall</th>
                      <th>F1</th>
                      <th>Improvement</th>
                    </tr>
                  </thead>
                  <tbody>
                    {sweepResults.slice(0, 20).map((result, idx) => {
                      const f1_improvement = result.f1 - sweepBaseline.f1;
                      const is_improvement = f1_improvement > 0.001;
                      const is_best = idx === 0;
                      const isExpanded = expandedRow === idx;

                      return (
                        <React.Fragment key={idx}>
                          <tr className={is_best ? "best" : is_improvement ? "improvement" : ""}>
                            <td className="config-cell">
                              <button
                                className="expand-btn"
                                onClick={() => setExpandedRow(isExpanded ? null : idx)}
                                style={{ cursor: "pointer", background: "none", border: "none", fontSize: "1.2em" }}
                              >
                                {isExpanded ? "▼" : "▶"}
                              </button>
                              <div className="config-params">
                                {result.config.min_prepoint_duration !== sweepBaseline.config.min_prepoint_duration && (
                                  <span>min_pre={result.config.min_prepoint_duration}</span>
                                )}
                                {result.config.min_post_duration !== sweepBaseline.config.min_post_duration && (
                                  <span>min_post={result.config.min_post_duration}</span>
                                )}
                                {result.config.max_post_proba !== sweepBaseline.config.max_post_proba && (
                                  <span>max_post={result.config.max_post_proba.toFixed(2)}</span>
                                )}
                                {result.config.absolute_threshold !== sweepBaseline.config.absolute_threshold && (
                                  <span>abs_thr={result.config.absolute_threshold.toFixed(2)}</span>
                                )}
                                {result.config.min_drop !== sweepBaseline.config.min_drop && (
                                  <span>min_drop={result.config.min_drop.toFixed(2)}</span>
                                )}
                                {result.config.min_gap !== sweepBaseline.config.min_gap && (
                                  <span>min_gap={result.config.min_gap}</span>
                                )}
                                {result.config.smoothing_window !== sweepBaseline.config.smoothing_window && (
                                  <span>smooth={result.config.smoothing_window}</span>
                                )}
                                {result.config.field_onset !== sweepBaseline.config.field_onset && (
                                  <span>field_onset={result.config.field_onset.toFixed(2)}</span>
                                )}
                              </div>
                            </td>
                            <td>{result.total_tp}</td>
                            <td>{result.total_fp}</td>
                            <td>{result.total_fn}</td>
                            <td>{(result.precision * 100).toFixed(1)}%</td>
                            <td>{(result.recall * 100).toFixed(1)}%</td>
                            <td className="f1-cell">{result.f1.toFixed(3)}</td>
                            <td className="improvement-cell">
                              {f1_improvement > 0 ? "+" : ""}{(f1_improvement * 1000).toFixed(0)}‰
                            </td>
                          </tr>
                          {isExpanded && (result.total_fn > 0 || result.total_fp > 0) && (
                            <tr className="expanded-row">
                              <td colSpan={8}>
                                <div className="causes-container">
                                  {result.total_fn > 0 && Object.values(result.fn_causes).some(c => c > 0) && (
                                    <div className="fn-causes-section">
                                      <h4>False Negative Root Causes</h4>
                                      <div className="fn-causes-list">
                                        {Object.entries(result.fn_causes)
                                          .filter(([_, count]) => count > 0)
                                          .map(([cause, count]) => (
                                            <div key={cause} className="fn-cause-item">
                                              <span className="cause-name">{cause.replace(/_/g, " ")}</span>
                                              <span className="cause-count">{count}</span>
                                            </div>
                                          ))}
                                      </div>
                                    </div>
                                  )}
                                  {result.total_fp > 0 && Object.values(result.fp_causes).some(c => c > 0) && (
                                    <div className="fp-causes-section">
                                      <h4>False Positive Root Causes</h4>
                                      <div className="fp-causes-list">
                                        {Object.entries(result.fp_causes)
                                          .filter(([_, count]) => count > 0)
                                          .map(([cause, count]) => (
                                            <div key={cause} className="fp-cause-item">
                                              <span className="cause-name">{cause.replace(/_/g, " ")}</span>
                                              <span className="cause-count">{count}</span>
                                            </div>
                                          ))}
                                      </div>
                                    </div>
                                  )}
                                </div>
                              </td>
                            </tr>
                          )}
                        </React.Fragment>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </div>
          )}
        </div>
      )}
    </div>
  );
};

export default GlobalEvaluation;
