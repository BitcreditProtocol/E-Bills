# E-bill The Flow Diagram

```mermaid
%%{
  init: {
    'theme': 'base',
    'themeVariables': {
      'primaryColor': '#FFFFFF',
      'primaryTextColor': '#000000',
      'primaryBorderColor': '#000000',
      'lineColor': '#000000',
      'secondaryColor': '#EEEEEE',
      'secondaryBorderColor': '#000000',
      'tertiaryColor': '#FFFFFF'
    }
  }
}%%

flowchart TD
    S[E-bill\nThe Flow] --> A(Alice Ltd. in London\nis short on cash)
    A --> |1. Proposes payment\nin three months\nvia e-bill| C(Charlie GmbH in Vienna\nhas long history with Alice)
    C --> |2. Agrees to get the\npayment via e-bill| A
    A --> |3. Draws and digitally signs| E1[E-bill of exchange\nInstructs Bob to pay 12.5 BTC to\nCharlie after three months]
    E1 -.-> E2[E-bill of exchange\nInstructs Bob to pay 12.5 BTC to\nCharlie after three months\nSigned by Charlie]
    E2 -.-> E3[E-bill of exchange\nInstructs Bob to pay 12.5 BTC to\nCharlie after three months\nSigned by Bob]
    C --> |4. Delivers the goods| A
    E1 ---> |5. Alice sends e-bill\nin exchange for goods| C
    C --> |6. Later...\nCharlie digitally signs\nthe acceptance request to Bob| E2
    E2 --> |7. Bob has to decide| BD{Bob Corporation in Boston\nOwes money to Alice\nAccepts to pay by e-bill?}
    BD --> |7a. Yes -> Digitally signs the\nacceptance of the e-bill| E3
    E3 --> |8a. Is held for awhile by| C
    C --> |9a. Later...\nWants to purchase goods\nand pay with e-bill to| D(Dave AG in Hamburg\nhas long history with Charlie)
    C --> |10a. Digitally signs the \nendorsement of the e-bill to Dave| E4[E-bill of exchange\nInstructs Bob to pay 12.5 BTC to\nCharlie after three months\nEndorsed to Dave\nSigned by Charlie]
    D --> |11a. Digitally signs a\npayment request to| BD
    BD --> |12a. Sends bitcoins for the e-bill\nto Dave's specific address| X[Dave's bitcoin address\nonly he can spend]
    D --> |13a. Sees the confirmed\nreceipt of the 12.5 BTC\nat his address| X
    X --> |.| E[End of flow]
    BD --> |7b. No| C
    C --> |8b. Request immediate\npayment from| A
    subgraph -=-=-
        S
        A
        C
        BD
        D
        E1
        E2
        E3
        E4
        X
        E
    end
```