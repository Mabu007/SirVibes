import type { ApprovalRequest } from "../lib/agent";

export function ApprovalDialog({
  request,
  onDecide,
}: {
  request: ApprovalRequest;
  onDecide: (approved: boolean) => void;
}) {
  const { evaluation } = request;
  return (
    <div className="overlay">
      <div className="dialog">
        <div className="dialog-title">{evaluation.title}</div>
        <pre className="dialog-detail">{evaluation.detail || request.tool}</pre>

        {evaluation.risks.length > 0 && (
          <ul className="risks">
            {evaluation.risks.map((r, i) => (
              <li key={i} className={`risk risk-${r.kind}`}>
                {r.message}
              </li>
            ))}
          </ul>
        )}

        <div className="dialog-actions">
          <button className="btn-deny" onClick={() => onDecide(false)}>
            Deny
          </button>
          <button className="btn-allow" onClick={() => onDecide(true)} autoFocus>
            Allow
          </button>
        </div>
      </div>
    </div>
  );
}
