package methods

import (
	"context"
	"fmt"

	"github.com/creachadair/jrpc2"
	"github.com/creachadair/jrpc2/handler"
	"github.com/stellar/go/support/log"
	"github.com/stellar/go/xdr"

	"github.com/stellar/soroban-tools/cmd/soroban-rpc/internal/db"
	"github.com/stellar/soroban-tools/cmd/soroban-rpc/internal/preflight"
)

type SimulateTransactionRequest struct {
	Transaction string `json:"transaction"`
}

type SimulateTransactionCost struct {
	CPUInstructions uint64 `json:"cpuInsns,string"`
	MemoryBytes     uint64 `json:"memBytes,string"`
}

// SimulateHostFunctionResult contains the simulation result of each HostFunction within the single InvokeHostFunctionOp allowed in a Transaction
type SimulateHostFunctionResult struct {
	Auth []string `json:"auth"`
	XDR  string   `json:"xdr"`
}

type SimulateTransactionResponse struct {
	Error           string                       `json:"error,omitempty"`
	TransactionData string                       `json:"transactionData"` // SorobanTransactionData XDR in base64
	Events          []string                     `json:"events"`          // DiagnosticEvent XDR in base64
	MinResourceFee  int64                        `json:"minResourceFee,string"`
	Results         []SimulateHostFunctionResult `json:"results,omitempty"` // an array of the individual host function call results
	Cost            SimulateTransactionCost      `json:"cost"`              // the effective cpu and memory cost of the invoked transaction execution.
	LatestLedger    int64                        `json:"latestLedger,string"`
}

type PreflightGetter interface {
	GetPreflight(ctx context.Context, readTx db.LedgerEntryReadTx, bucketListSize uint64, sourceAccount xdr.AccountId, opBody xdr.OperationBody, footprint xdr.LedgerFootprint) (preflight.Preflight, error)
}

// NewSimulateTransactionHandler returns a json rpc handler to run preflight simulations
func NewSimulateTransactionHandler(logger *log.Entry, ledgerEntryReader db.LedgerEntryReader, ledgerReader db.LedgerReader, getter PreflightGetter) jrpc2.Handler {

	return handler.New(func(ctx context.Context, request SimulateTransactionRequest) SimulateTransactionResponse {
		var txEnvelope xdr.TransactionEnvelope
		if err := xdr.SafeUnmarshalBase64(request.Transaction, &txEnvelope); err != nil {
			logger.WithError(err).WithField("request", request).
				Info("could not unmarshal simulate transaction envelope")
			return SimulateTransactionResponse{
				Error: "Could not unmarshal transaction",
			}
		}
		if len(txEnvelope.Operations()) != 1 {
			return SimulateTransactionResponse{
				Error: "Transaction contains more than one operation",
			}
		}
		op := txEnvelope.Operations()[0]

		var sourceAccount xdr.AccountId
		if opSourceAccount := op.SourceAccount; opSourceAccount != nil {
			sourceAccount = opSourceAccount.ToAccountId()
		} else {
			sourceAccount = txEnvelope.SourceAccount().ToAccountId()
		}

		footprint := xdr.LedgerFootprint{}
		switch op.Body.Type {
		case xdr.OperationTypeInvokeHostFunction:
		case xdr.OperationTypeBumpFootprintExpiration, xdr.OperationTypeRestoreFootprint:
			if txEnvelope.Type != xdr.EnvelopeTypeEnvelopeTypeTx && txEnvelope.V1.Tx.Ext.V != 1 {
				return SimulateTransactionResponse{
					Error: "To perform a SimulateTransaction for BumpFootprintExpiration or RestoreFootprint operations, SorobanTransactionData must be provided",
				}
			}
			footprint = txEnvelope.V1.Tx.Ext.SorobanData.Resources.Footprint
		default:
			return SimulateTransactionResponse{
				Error: "Transaction contains unsupported operation type: " + op.Body.Type.String(),
			}
		}

		readTx, err := ledgerEntryReader.NewCachedTx(ctx)
		if err != nil {
			return SimulateTransactionResponse{
				Error: "Cannot create read transaction",
			}
		}
		defer func() {
			_ = readTx.Done()
		}()
		latestLedger, err := readTx.GetLatestLedgerSequence()
		if err != nil {
			return SimulateTransactionResponse{
				Error: err.Error(),
			}
		}
		bucketListSize, err := getBucketListSize(ctx, ledgerReader, latestLedger)
		if err != nil {
			return SimulateTransactionResponse{
				Error: err.Error(),
			}
		}

		result, err := getter.GetPreflight(ctx, readTx, bucketListSize, sourceAccount, op.Body, footprint)
		if err != nil {
			return SimulateTransactionResponse{
				Error:        err.Error(),
				LatestLedger: int64(latestLedger),
			}
		}

		return SimulateTransactionResponse{
			Results: []SimulateHostFunctionResult{
				{
					XDR:  result.Result,
					Auth: result.Auth,
				},
			},
			Events:          result.Events,
			TransactionData: result.TransactionData,
			MinResourceFee:  result.MinFee,
			Cost: SimulateTransactionCost{
				CPUInstructions: result.CPUInstructions,
				MemoryBytes:     result.MemoryBytes,
			},
			LatestLedger: int64(latestLedger),
		}
	})
}

func getBucketListSize(ctx context.Context, ledgerReader db.LedgerReader, latestLedger uint32) (uint64, error) {
	// obtain bucket size
	var closeMeta, ok, err = ledgerReader.GetLedger(ctx, latestLedger)
	if err != nil {
		return 0, err
	}
	if !ok {
		return 0, fmt.Errorf("missing meta for latest ledger (%d)", latestLedger)
	}
	if closeMeta.V != 2 {
		return 0, fmt.Errorf("latest ledger (%d) meta has unexpected verion (%d)", latestLedger, closeMeta.V)
	}
	return uint64(closeMeta.V2.TotalByteSizeOfBucketList), nil
}
