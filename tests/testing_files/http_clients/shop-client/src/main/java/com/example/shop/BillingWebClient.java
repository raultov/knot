package com.example.shop;

/** WebClient whose base URL is set once, by the builder of this file. */
@Component
public class BillingWebClient {
    private final WebClient webClient;

    public BillingWebClient(WebClient.Builder builder, @Value("${billing.url}") String billingUrl) {
        this.webClient = builder.baseUrl(billingUrl).build();
    }

    public Mono<Invoice> invoice(String number) {
        return webClient.get().uri("/invoices/{number}", number).retrieve().bodyToMono(Invoice.class);
    }

    public Mono<Refund> refund(Refund refund) {
        return webClient.post()
            .uri(uriBuilder -> uriBuilder.path("/refunds").queryParam("notify", true).build())
            .bodyValue(refund)
            .retrieve()
            .bodyToMono(Refund.class);
    }
}
