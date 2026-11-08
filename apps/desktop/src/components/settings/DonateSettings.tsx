import { isTauri } from '@tauri-apps/api/core';
import { Check, Coffee, Copy, ExternalLink, Heart, QrCode, Wallet } from 'lucide-react';
import { useState, type MouseEvent } from 'react';
import { DONATION_CONFIG } from './donationConfig';

type Notice = { kind: 'success' | 'error'; text: string };
type CopyField = 'phone' | 'memo';

export function DonateSettings() {
  const [notice, setNotice] = useState<Notice | null>(null);
  const [copiedField, setCopiedField] = useState<CopyField | null>(null);

  function openSupportLink(event: MouseEvent<HTMLAnchorElement>, url: string) {
    if (!isTauri()) return;
    event.preventDefault();
    setNotice(null);
    void (async () => {
      try {
        const { openUrl } = await import('@tauri-apps/plugin-opener');
        await openUrl(url);
      } catch {
        setNotice({
          kind: 'error',
          text: `Could not open the browser. Visit ${url} manually.`,
        });
      }
    })();
  }

  async function copyToClipboard(field: CopyField, value: string) {
    setNotice(null);
    setCopiedField(null);
    try {
      await navigator.clipboard.writeText(value);
      setCopiedField(field);
      setNotice({
        kind: 'success',
        text: `${field === 'phone' ? 'MoMo phone number' : 'Transfer note'} copied.`,
      });
    } catch {
      setNotice({ kind: 'error', text: 'Could not copy. Select and copy the text manually.' });
    }
  }

  return (
    <section className="settings-section donate-settings" aria-labelledby="donate-settings-title">
      <div className="donate-heading">
        <span className="donate-heading-icon" aria-hidden="true">
          <Heart size={19} />
        </span>
        <div>
          <h3 id="donate-settings-title">Support Sky Auto Player</h3>
          <p className="settings-note">
            Free and open-source. Donations help support continued maintenance and development.
          </p>
        </div>
      </div>

      <div className="donate-provider-grid">
        <div className="donate-provider-card">
          <div className="donate-provider-heading">
            <Coffee size={17} aria-hidden="true" />
            <div>
              <h4>Ko-fi</h4>
              <p>Support the project</p>
            </div>
          </div>
          <a
            className="button donate-external-link"
            href={DONATION_CONFIG.koFiUrl}
            aria-label="Open Ko-fi in browser"
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => openSupportLink(event, DONATION_CONFIG.koFiUrl)}
          >
            Open <ExternalLink size={14} aria-hidden="true" />
          </a>
        </div>
        <div className="donate-provider-card">
          <div className="donate-provider-heading">
            <Heart size={17} aria-hidden="true" />
            <div>
              <h4>PayPal</h4>
              <p>International support</p>
            </div>
          </div>
          <a
            className="button donate-external-link"
            href={DONATION_CONFIG.paypalUrl}
            aria-label="Open PayPal in browser"
            target="_blank"
            rel="noopener noreferrer"
            onClick={(event) => openSupportLink(event, DONATION_CONFIG.paypalUrl)}
          >
            Open <ExternalLink size={14} aria-hidden="true" />
          </a>
        </div>
      </div>

      <div className="donate-bank-card">
        <div className="donate-provider-heading">
          <Wallet size={17} aria-hidden="true" />
          <h4>MoMo · Vietnam</h4>
        </div>
        <p className="settings-note">
          Send to this MoMo wallet using its phone number. You can also select MoMo as the recipient
          bank in supported banking apps. Always verify the recipient before confirming.
        </p>

        <div className="donate-bank-content">
          {DONATION_CONFIG.momoQrImagePath ? (
            <div className="donate-qr-frame">
              <img
                src={DONATION_CONFIG.momoQrImagePath}
                alt="Official MoMo receive QR for the listed wallet"
                width={172}
                height={172}
              />
            </div>
          ) : (
            <div className="donate-qr-placeholder" role="group" aria-label="Official MoMo QR not yet configured">
              <QrCode size={28} aria-hidden="true" />
              <strong>MoMo receive QR</strong>
              <span>Not available yet</span>
              <span className="donate-qr-hint">Use the wallet phone number for now.</span>
            </div>
          )}

          <div className="donate-bank-information">
            <dl className="donate-bank-details">
              <div>
                <dt>Account holder</dt>
                <dd>{DONATION_CONFIG.momoAccountHolder}</dd>
              </div>
              <div>
                <dt>MoMo wallet phone</dt>
                <dd className="donate-bank-account">{DONATION_CONFIG.momoPhoneNumber}</dd>
              </div>
              <div>
                <dt>Receiving ID shown in MoMo</dt>
                <dd className="donate-bank-account donate-receiving-id">
                  {DONATION_CONFIG.momoReceivingId}
                </dd>
              </div>
              <div>
                <dt>Suggested transfer note</dt>
                <dd>{DONATION_CONFIG.transferMemo}</dd>
              </div>
            </dl>
            <div className="donate-copy-actions">
              <button
                className="button donate-copy-button"
                type="button"
                onClick={() => void copyToClipboard('phone', DONATION_CONFIG.momoPhoneNumber)}
              >
                {copiedField === 'phone' ? (
                  <Check size={14} aria-hidden="true" />
                ) : (
                  <Copy size={14} aria-hidden="true" />
                )}
                Copy phone
              </button>
              <button
                className="button donate-copy-button"
                type="button"
                onClick={() => void copyToClipboard('memo', DONATION_CONFIG.transferMemo)}
              >
                {copiedField === 'memo' ? (
                  <Check size={14} aria-hidden="true" />
                ) : (
                  <Copy size={14} aria-hidden="true" />
                )}
                Copy note
              </button>
            </div>
          </div>
        </div>
        {!DONATION_CONFIG.momoQrImagePath && (
          <p className="settings-note donate-qr-note">
            The official MoMo QR has not been added. The PSP receiving ID is shown for reference
            only; do not use it as a bank account number without verifying the recipient in MoMo.
          </p>
        )}
      </div>

      {notice && (
        <p
          className={notice.kind === 'error' ? 'inline-error' : 'settings-note'}
          role={notice.kind === 'error' ? 'alert' : 'status'}
        >
          {notice.text}
        </p>
      )}
      <p className="settings-note donate-disclaimer">
        All donations are optional. Sky Auto Player never handles payments or confirms transfers.
        You can also{' '}
        <a
          href={DONATION_CONFIG.githubUrl}
          target="_blank"
          rel="noopener noreferrer"
          onClick={(event) => openSupportLink(event, DONATION_CONFIG.githubUrl)}
        >
          star on GitHub <ExternalLink size={12} aria-hidden="true" />
        </a>
        .
      </p>
    </section>
  );
}
