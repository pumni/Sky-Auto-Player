import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DonateSettings } from './DonateSettings';
import { DONATION_CONFIG } from './donationConfig';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('DonateSettings', () => {
  it('shows Ko-fi, PayPal and the offline MBBank VietQR without outdated wallet data', () => {
    render(<DonateSettings />);
    expect(screen.getByRole('region', { name: 'Support Sky Auto Player' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: 'Open Ko-fi in browser' })).toHaveAttribute(
      'href',
      DONATION_CONFIG.koFiUrl,
    );
    expect(screen.getByRole('link', { name: 'Open PayPal in browser' })).toHaveAttribute(
      'href',
      DONATION_CONFIG.paypalUrl,
    );
    expect(screen.getByRole('link', { name: /star on GitHub/ })).toHaveAttribute(
      'href',
      DONATION_CONFIG.githubUrl,
    );

    expect(
      screen
        .getByRole('heading', { name: 'PayPal' })
        .closest('.donate-provider-heading')
        ?.querySelector('svg'),
    ).toHaveClass('lucide-wallet-cards');

    expect(screen.getByRole('heading', { name: 'Bank transfer · VietQR' })).toBeInTheDocument();
    expect(screen.getAllByText(DONATION_CONFIG.bankName)).toHaveLength(1);
    expect(screen.getByText(DONATION_CONFIG.bankName)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.accountHolder)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.accountNumber)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.transferMemo)).toBeInTheDocument();
    expect(screen.queryByText(/No fixed amount|· VND/)).not.toBeInTheDocument();
    expect(document.querySelector('.donate-qr-card')).toBeNull();
    const guidance = screen.getByText(
      'Scan the QR or copy the details. Check the recipient before sending.',
    );
    expect(guidance).toHaveClass('donate-bank-guidance');
    expect(guidance.parentElement).toHaveClass('donate-bank-information');
    expect(document.querySelector('.donate-bank-card > .settings-note')).toBeNull();
    expect(screen.getByRole('img', { name: 'VietQR bank transfer code' })).toHaveAttribute(
      'src',
      DONATION_CONFIG.vietQrImagePath,
    );
    const copyRows = document.querySelectorAll('.donate-bank-copy-row');
    expect(copyRows).toHaveLength(2);
    expect(copyRows[0]).toContainElement(screen.getByRole('button', { name: 'Copy account' }));
    expect(copyRows[1]).toContainElement(screen.getByRole('button', { name: 'Copy note' }));
    const githubLink = screen.getByRole('link', { name: /star on GitHub/ });
    expect(githubLink.querySelector('span')).toHaveTextContent('star on GitHub');
    expect(githubLink.querySelector('svg')).toHaveClass('lucide-external-link');
    expect(screen.queryByText(/MoMo|PSP2603710800000415|Techcombank/i)).not.toBeInTheDocument();
  });

  it('copies the bank account number and transfer note', async () => {
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText },
    });
    try {
      render(<DonateSettings />);
      fireEvent.click(screen.getByRole('button', { name: 'Copy account' }));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith(DONATION_CONFIG.accountNumber));
      expect(screen.getByRole('status')).toHaveTextContent('Account number copied.');

      fireEvent.click(screen.getByRole('button', { name: 'Copy note' }));
      await waitFor(() => expect(writeText).toHaveBeenCalledWith(DONATION_CONFIG.transferMemo));
      expect(screen.getByRole('status')).toHaveTextContent('Transfer note copied.');
    } finally {
      if (previousClipboard) {
        Object.defineProperty(navigator, 'clipboard', previousClipboard);
      } else {
        Reflect.deleteProperty(navigator, 'clipboard');
      }
    }
  });

  it('leaves bank details selectable when clipboard access is denied', async () => {
    const previousClipboard = Object.getOwnPropertyDescriptor(navigator, 'clipboard');
    Object.defineProperty(navigator, 'clipboard', {
      configurable: true,
      value: { writeText: vi.fn().mockRejectedValue(new Error('denied')) },
    });
    try {
      render(<DonateSettings />);
      fireEvent.click(screen.getByRole('button', { name: 'Copy account' }));
      expect(await screen.findByRole('alert')).toHaveTextContent('Could not copy');
      expect(screen.getByText(DONATION_CONFIG.accountNumber)).toBeInTheDocument();
    } finally {
      if (previousClipboard) {
        Object.defineProperty(navigator, 'clipboard', previousClipboard);
      } else {
        Reflect.deleteProperty(navigator, 'clipboard');
      }
    }
  });
});
