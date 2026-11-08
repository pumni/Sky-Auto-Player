import { cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { DonateSettings } from './DonateSettings';
import { DONATION_CONFIG } from './donationConfig';

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
});

describe('DonateSettings', () => {
  it('shows the three donation methods and real, restricted destinations', () => {
    render(<DonateSettings />);
    expect(screen.getByRole('region', { name: 'Support Sky Auto Player' })).toBeInTheDocument();
    expect(screen.getByRole('link', { name: /Open Ko-fi/ })).toHaveAttribute(
      'href',
      DONATION_CONFIG.koFiUrl,
    );
    expect(screen.getByRole('link', { name: /Open PayPal/ })).toHaveAttribute(
      'href',
      DONATION_CONFIG.paypalUrl,
    );
    expect(screen.getByRole('link', { name: /star the project on GitHub/ })).toHaveAttribute(
      'href',
      DONATION_CONFIG.githubUrl,
    );
    expect(screen.getByAltText(/VietQR for the listed Techcombank account/)).toHaveAttribute(
      'src',
      DONATION_CONFIG.vietQrImagePath,
    );
    expect(screen.getByText(DONATION_CONFIG.accountHolder)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.accountNumber)).toBeInTheDocument();
    expect(screen.getByText(DONATION_CONFIG.transferMemo)).toBeInTheDocument();
    expect(screen.getByText(/Enter any amount/)).toBeInTheDocument();
  });

  it('copies the account number and transfer note', async () => {
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

  it('reports clipboard permission failures without hiding bank details', async () => {
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
